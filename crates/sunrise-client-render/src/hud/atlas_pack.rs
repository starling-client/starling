use super::*;

impl HudAtlas {
    /// Pack against a skyline so space below shorter neighbours stays usable.
    /// Pixel UI must not be reduced merely because tall shelves leave gaps.
    #[cfg(any(target_os = "android", test))]
    fn mobile_layout(images: &[PackedImage], maximum: u32) -> Result<(u32, Vec<(u32, u32)>, u32)> {
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
    fn fit_mobile_atlas(
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
    fn layout_within(
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
    /// Shelf placements of `images` (tallest first) in the narrowest atlas
    /// width from 4096 up that keeps the atlas, glyph rows included, within
    /// `max` texels a side; and the first row free for glyphs. High-resolution
    /// packs fill a 4096-wide atlas past what GPUs allow (Faithful 64x needs
    /// 32768 rows); vanilla's stitcher fails the reload when its atlases don't
    /// fit, and the packs are then turned off (`Minecraft.clearResourcePacksOnError`).
    /// A width where the plain shelves (each image after the last) leave too
    /// much unused is tried again with each image in the first shelf with
    /// room: a pack of thousands of mixed sizes (server packs with custom
    /// items) fits where the shelves' ragged ends would not.
    pub(super) fn layout(images: &[PackedImage], max: u32) -> Result<(u32, Vec<(u32, u32)>, u32)> {
        // Starling: a phone's small atlas is packed against a skyline (`mobile_layout`).
        #[cfg(target_os = "android")]
        if max <= 4096 {
            return Self::mobile_layout(images, max);
        }
        let mut width = 4096.min(max);
        loop {
            let wide_enough = images.iter().all(|i| i.1 < width);
            if wide_enough {
                for first_fit in [false, true] {
                    let (placements, glyph_region) = Self::shelves(images, width, first_fit);
                    if glyph_region + GLYPH_ROWS <= max {
                        return Ok((width, placements, glyph_region));
                    }
                }
            }
            if width >= max {
                let texels: u64 = images.iter().map(|i| u64::from(i.1) * u64::from(i.2)).sum();
                return Err(format!(
                    "the textures ({} images, {texels} texels) do not fit a {max}x{max} atlas",
                    images.len()
                )
                .into());
            }
            width = (width * 2).min(max);
        }
    }
    /// `layout`'s shelves at one width: the placements and the first row below
    /// them. `first_fit` puts an image in the first shelf that is tall enough
    /// and has room, instead of only the last one.
    fn shelves(images: &[PackedImage], width: u32, first_fit: bool) -> (Vec<(u32, u32)>, u32) {
        // Each shelf: its top row, its height (its first image's) and the next free column.
        let mut shelves: Vec<(u32, u32, u32)> = Vec::new();
        let mut placements = Vec::with_capacity(images.len());
        for (_, w, h, _) in images {
            let open = if first_fit {
                shelves.iter().position(|s| s.1 >= *h && s.2 + w < width)
            } else {
                shelves
                    .len()
                    .checked_sub(1)
                    .filter(|&i| shelves[i].1 >= *h && shelves[i].2 + w < width)
            };
            let i = open.unwrap_or_else(|| {
                let top = shelves.last().map_or(0, |s| s.0 + s.1 + 1);
                shelves.push((top, *h, 0));
                shelves.len() - 1
            });
            placements.push((shelves[i].2, shelves[i].0));
            shelves[i].2 += w + 1;
        }
        let glyph_region = shelves.last().map_or(1, |s| s.0 + s.1 + 1);
        (placements, glyph_region)
    }
    /// Pack the textures under these folders of `textures/`, and for the
    /// `full` atlas the generated sprites and the run-time slots too.
    pub(super) fn pack(
        root: &ResourceSource,
        defaults: &ResourceSource,
        folders: &[&str],
        full: bool,
        pool: bool,
    ) -> Result<Self> {
        let textures = "assets/minecraft/textures";
        let mut files = Vec::new();
        for folder in folders {
            walk(root, &format!("{textures}/{folder}"), &mut files);
        }
        if folders.contains(&"font") {
            let mut seen: HashSet<String> = files.iter().cloned().collect();
            files.extend(
                bitmap_font_textures(root)
                    .into_iter()
                    .filter(|p| seen.insert(p.clone())),
            );
        }
        // Native and content resources may use any namespace in the resolved pack.
        for namespace in root
            .namespaces("assets")
            .into_iter()
            .filter(|namespace| namespace != "minecraft")
        {
            for folder in ["entity", "mob_effect", "particle", "gui"] {
                if ((folder == "entity" || folder == "mob_effect") && folders.contains(&folder))
                    || (folder == "particle" && folders.contains(&"particle"))
                    || (folder == "gui" && full)
                {
                    walk(
                        root,
                        &format!("assets/{namespace}/textures/{folder}"),
                        &mut files,
                    );
                }
            }
        }
        files.retain(|p| !menu_pictures::menu_picture_only(p));
        // OptiFine random entity variants, under the names their scan gives them (those
        // outside `textures/` are `<ns>:optifine/random/...`).
        let random_textures = if folders.contains(&"entity") {
            sunrise_optifine::random_textures::RandomTextures::scan(root)
        } else {
            Default::default()
        };
        let blinks = if folders.contains(&"entity") {
            sunrise_optifine::blink::Blinks::scan(root, &random_textures)
        } else {
            Default::default()
        };
        let emissive = std::sync::Arc::new(if folders.contains(&"entity") {
            let mut e = sunrise_optifine::emissive::Emissive::scan(root, &random_textures);
            e.pair_beside(root, blinks.files());
            e
        } else {
            Default::default()
        });
        let mut named: HashMap<String, String> = HashMap::new();
        let extra = random_textures
            .files()
            .chain(emissive.files())
            .chain(blinks.files());
        for (file, name) in extra {
            named.insert(file.to_owned(), name.to_owned());
        }
        if !named.is_empty() {
            // A pack's variants can't grow the shared atlas past what a GPU takes: every variant
            // (those beside their texture too) counts, in name order, and those past the budget
            // stay out (their mobs wear the texture itself), not the whole pack.
            // Overlays and blink frames only after every variant.
            let overlays: HashSet<String> = emissive
                .files()
                .chain(blinks.files())
                .map(|(f, _)| f.to_owned())
                .collect();
            let kept = budgeted(root, named.keys(), &overlays, RANDOM_TEXELS);
            if kept.len() < named.len() {
                log::warn!(
                    "{} random entity textures left out of the atlas: past its budget",
                    named.len() - kept.len()
                );
            }
            // A dropped variant beside its texture takes its walked overlay and blink frames
            // (and theirs) with it: nothing would draw them.
            let png = format!("{}.png", emissive.suffix);
            let png = png.as_str();
            let orphans: HashSet<String> = named
                .keys()
                .filter(|p| !kept.contains(*p))
                .filter_map(|p| p.strip_suffix(".png").map(str::to_owned))
                .flat_map(|stem| {
                    ["", "_blink", "_blink2"]
                        .into_iter()
                        .flat_map(move |frame| {
                            [format!("{stem}{frame}.png"), format!("{stem}{frame}{png}")]
                        })
                })
                .filter(|p| !named.contains_key(p))
                .collect();
            files.retain(|p| (!named.contains_key(p) || kept.contains(p)) && !orphans.contains(p));
            // Those beside their texture are walked already.
            let walked: HashSet<&String> = files.iter().collect();
            let mut extra: Vec<String> = kept
                .iter()
                .filter(|p| !walked.contains(p))
                .cloned()
                .collect();
            extra.sort();
            files.extend(extra);
        }
        // A texture's LabPBR maps, as decoded (width, height, RGBA8), for shader packs.
        type Maps = [Option<(u32, u32, Vec<u8>)>; 2];
        let read_maps = full && crate::sprites::labpbr_maps();
        type Decoded = (
            PackedImage,
            Option<TextureFlags>,
            Option<(u32, u32)>,
            bool,
            Option<Box<Maps>>,
        );
        let decode_one = |path: &String| -> Option<Decoded> {
            let name = if let Some(name) = named.get(path) {
                name.clone()
            } else {
                let name = path.strip_suffix(".png")?;
                if let Some(relative) = name.strip_prefix("assets/minecraft/textures/") {
                    relative.to_owned()
                } else {
                    let (namespace, relative) =
                        name.strip_prefix("assets/")?.split_once("/textures/")?;
                    format!("{namespace}:{relative}")
                }
            };
            // LabPBR's maps beside a texture are the world's (sprites.rs), not images to draw.
            if let Some(base) = name.strip_suffix("_n").or_else(|| name.strip_suffix("_s"))
                && root.contains(&crate::atlas_sources::texture_resource(base, "textures"))
            {
                return None;
            }
            let image = decode_resource(root, path)
                .map_err(|e| log::warn!("skipped GUI texture {}: {e}", path))
                .ok()?;
            let mcmeta = root.read_to_string(&format!("{path}.mcmeta")).ok();
            let animation = animation_of(mcmeta.as_deref());
            // An animated strip keeps its first frame.
            let frames =
                animation.is_some() || name.starts_with("block/") || name.starts_with("item/");
            let (w, h, pixels) = if frames {
                first_frame(image, animation.as_ref())
                    .map_err(|e| log::warn!("skipped GUI texture {}: {e}", path))
                    .ok()?
            } else {
                image
            };
            // The default texture's size, when a pack's differs from it.
            let gui_size = png_size(
                defaults,
                &crate::atlas_sources::texture_resource(&name, "textures"),
            )
            .map(|(dw, dh)| {
                let side = dw.min(dh);
                if frames { (side, side) } else { (dw, dh) }
            })
            .filter(|&size| size != (w, h));
            let flags = mcmeta.as_deref().and_then(TextureFlags::from_mcmeta);
            let animated = frames && animation.is_some();
            // Vanilla particle atlases accept sprites of any size.
            let particle = name.starts_with("particle/") || name.contains(":particle/");
            let (gw, gh) = if particle {
                (0, 0)
            } else {
                gui_size.unwrap_or((w, h))
            };
            let maps = read_maps
                .then(|| {
                    let stem = path.strip_suffix(".png")?;
                    let maps: Maps = ["_n", "_s"]
                        .map(|suffix| decode_resource(root, &format!("{stem}{suffix}.png")).ok());
                    maps.iter().any(Option::is_some).then(|| Box::new(maps))
                })
                .flatten();
            (gw <= 1024
                && (gh <= 512
                    || matches!(
                        name.as_str(),
                        "gui/title/background/panorama_0"
                            | "gui/title/background/panorama_2"
                            | "gui/title/background/panorama_3"
                            | "gui/presets/isles"
                    )
                    || name.starts_with("font/")))
            .then_some(((name, w, h, pixels), flags, gui_size, animated, maps))
        };
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        let chunk = files.len().div_ceil(threads).max(1);
        let decoded: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = files
                .chunks(chunk)
                .map(|part| {
                    scope.spawn(move || part.iter().filter_map(decode_one).collect::<Vec<_>>())
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });
        let mut flags = HashMap::new();
        let mut gui_sizes = HashMap::new();
        let mut animated: Vec<String> = Vec::new();
        let mut images: Vec<PackedImage> = Vec::with_capacity(decoded.len());
        let mut maps: HashMap<String, Box<Maps>> = HashMap::new();
        for (image, image_flags, gui_size, is_animated, image_maps) in decoded {
            if let Some(m) = image_maps {
                maps.insert(image.0.clone(), m);
            }
            if let Some(f) = image_flags {
                flags.insert(image.0.clone(), f);
            }
            if let Some(size) = gui_size {
                gui_sizes.insert(image.0.clone(), size);
            }
            if is_animated {
                animated.push(image.0.clone());
            }
            images.push(image);
        }
        // A registered atlas's sprites (`ns:gui/atlases/...`) are packed under keys of their own,
        // so no model's or GUI entry's sprite of the same name can displace or be taken for one.
        for image in &mut images {
            let path = image.0.split_once(':').map_or(image.0.as_str(), |(_, p)| p);
            if !path.starts_with("gui/atlases/") {
                continue;
            }
            let key = format!("{REGISTERED_KEY}{}", image.0);
            let old = std::mem::replace(&mut image.0, key.clone());
            if let Some(value) = flags.remove(&old) {
                flags.insert(key.clone(), value);
            }
            if let Some(value) = gui_sizes.remove(&old) {
                gui_sizes.insert(key.clone(), value);
            }
            if let Some(value) = maps.remove(&old) {
                maps.insert(key.clone(), value);
            }
            for name in animated.iter_mut().filter(|name| **name == old) {
                name.clone_from(&key);
            }
        }
        let mut cit_names = HashSet::new();
        let mut gui_files = HashMap::new();
        let mut gui_names = Vec::new();
        let mut non_gui = HashSet::new();
        if full {
            let found = atlas_sprites(root, &images);
            let dropped = found.dropped;
            images.retain(|image| !dropped.contains(&image.0));
            flags.retain(|name, _| !dropped.contains(name));
            gui_sizes.retain(|name, _| !dropped.contains(name));
            maps.retain(|name, _| !dropped.contains(name));
            animated.retain(|name| !dropped.contains(name));
            images.extend(found.images);
            animated.extend(found.animated);
            flags.extend(found.flags);
            gui_files = found.gui_files;
            gui_names = found.gui;
            non_gui = found.non_gui;
            for (image, image_flags, is_animated) in cit_sprites(root, &images) {
                cit_names.insert(image.0.clone());
                if let Some(f) = image_flags {
                    flags.insert(image.0.clone(), f);
                }
                if is_animated {
                    animated.push(image.0.clone());
                }
                images.push(image);
            }
        }
        // Downloaded Android controls can arrive after sunrise-data.bin was built.
        // Read this small overlay from disk too, without walking every prepared asset.
        if full {
            let mut existing: HashSet<String> = images.iter().map(|i| i.0.clone()).collect();
            for (base, folder, prefix) in [
                (root, "assets/minecraft/textures/gui/bedrock", "gui/bedrock"),
                (
                    root,
                    "assets/starling/textures/gui/sprites/touch",
                    "starling:gui/sprites/touch",
                ),
                (
                    defaults,
                    "assets/minecraft/textures/gui/bedrock",
                    "gui/bedrock",
                ),
                (
                    defaults,
                    "assets/starling/textures/gui/sprites/touch",
                    "starling:gui/sprites/touch",
                ),
            ] {
                let dir = format!("{folder}/");
                for file in base.list(&dir) {
                    let Some(stem) = file
                        .strip_prefix(&dir)
                        .and_then(|relative| relative.strip_suffix(".png"))
                    else {
                        continue;
                    };
                    let name = format!("{prefix}/{stem}");
                    // From the disk: downloaded after the folder's data file was written.
                    if !existing.contains(&name)
                        && let Ok(bytes) = base.read_loose(&file)
                        && let Ok((w, h, pixels)) = decode_bytes(&bytes)
                    {
                        existing.insert(name.clone());
                        images.push((name, w, h, pixels));
                    }
                }
            }
        }
        #[cfg(target_os = "android")]
        if full {
            log::info!(
                "Android control atlas: {} original textures (root {:?}, defaults {:?})",
                images
                    .iter()
                    .filter(|i| i.0.starts_with("gui/bedrock/"))
                    .count(),
                root,
                defaults
            );
        }
        // The menus' atlas too: the touch layout editor works before a world.
        let touch = builtin_touch_sprites(&images);
        images.extend(touch);
        images.extend(android_utility_sprites(&images));
        // Starling: the title screen's "JAVA EDITION" banner reads "RUST EDITION", as before
        // the sync with Sunrise (which draws vanilla's banner).
        if let Some(edition) = images
            .iter_mut()
            .find(|i| i.0 == crate::title_edition::SPRITE)
            && let Some(pixels) =
                crate::title_edition::rust_edition(edition.1, edition.2, &edition.3)
        {
            edition.3 = pixels;
        }
        images.push(("sunrise/white".into(), 4, 4, vec![255; 64]));
        // The controller's art, unless a pack brought its own (walked above, as a mod's GUI
        // textures are).
        for image in crate::pad_glyph::images().chain(crate::mark_sprite::image()) {
            if !images.iter().any(|i| i.0 == image.0) {
                images.push(image);
            }
        }
        if full {
            // Official Recipes artwork from Just Enough Items, under MIT.
            for (name, png) in crate::jei_sprites::SPRITES {
                let (width, height, rgba) = decode_bytes(png)?;
                images.push((format!("sunrise/jei/{name}"), width, height, rgba));
            }
            Self::slots(&mut images, crate::trims::slot_size(root), pool);
            // MissingTextureAtlasSprite.generateMissingImage: 16x16, magenta
            // and black quarters.
            let missing = (0..256)
                .flat_map(|i| match (i / 16 < 8) ^ (i % 16 < 8) {
                    true => [248, 0, 248, 255],
                    false => [0, 0, 0, 255],
                })
                .collect();
            images.push((MISSING_SPRITE.into(), 16, 16, missing));
        }
        let limit = atlas_limit();
        let mut full_sizes = HashMap::new();
        #[cfg(target_os = "android")]
        if limit < MAX_ATLAS_SIZE {
            Self::fit_mobile_atlas(&mut images, &mut full_sizes, &flags, limit)?;
        }
        // Blurred sprites are sampled linearly in the atlas (screen.wgsl): a
        // border wrapped from their own edges keeps the filter off their
        // neighbours, as repeat addressing of their own texture would.
        let mut bordered = std::collections::HashSet::new();
        for image in &mut images {
            if flags.get(&image.0).is_some_and(|f: &TextureFlags| f.blur) {
                *image = wrap_border(image);
                bordered.insert(image.0.clone());
            }
        }
        if full {
            let (width, height, rgba) =
                decode_bytes(include_bytes!("../jei_recipes/config_button.png"))?;
            images.push(("sunrise/recipes/settings".to_owned(), width, height, rgba));
        }
        images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        let (width, placements, glyph_region) = Self::layout_within(
            &mut images,
            &mut animated,
            &cit_names,
            &bordered,
            limit,
            &mut full_sizes,
        )
        .or_else(|error| -> Result<_> {
            let models = reducible_model_sprites(root, &images, &bordered);
            let layout =
                Self::layout_reducing_models(&mut images, &mut gui_sizes, &models, MAX_ATLAS_SIZE)?;
            log::warn!("reduced HUD model texture resolution to keep the resource pack: {error}");
            Ok(layout)
        })?;
        // A multiple of 1024 rather than a power of two: the default resources
        // need 4096x5120 (8192 rows before), Faithful 32x 4096x12288 (16384).
        let height = (glyph_region + GLYPH_ROWS)
            .next_multiple_of(1024)
            .min(limit);
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut sprites = HashMap::new();
        let mut translucent = HashSet::new();
        for ((name, w, h, data), (px, py)) in images.iter().zip(placements) {
            if data
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] != 0 && p[3] != 255)
            {
                translucent.insert(name.clone());
            }
            if !data.is_empty() {
                for row in 0..*h {
                    let src = &data[(row * w * 4) as usize..((row + 1) * w * 4) as usize];
                    let dst = (((py + row) * width + px) * 4) as usize;
                    pixels[dst..dst + (w * 4) as usize].copy_from_slice(src);
                }
            }
            let b = u32::from(bordered.contains(name));
            let (px, py, w, h) = (px + b, py + b, w - 2 * b, h - 2 * b);
            let (gw, gh) = gui_sizes
                .get(name)
                .or(full_sizes.get(name))
                .copied()
                .unwrap_or((w, h));
            sprites.insert(
                name.clone(),
                Rect {
                    u0: px as f32 / width as f32,
                    v0: py as f32 / height as f32,
                    u1: (px + w) as f32 / width as f32,
                    v1: (py + h) as f32 / height as f32,
                    w: gw,
                    h: gh,
                },
            );
        }
        // Every animated sprite animates: the packed strip is the raw first
        // frame, while the mcmeta's frame list may start anywhere, so each
        // animation's frame(0) overwrites its atlas rect before drawing.
        let mut loaded = crate::sprites::Sprites::new(root.clone());
        let mut sprite_animations = Vec::new();
        animated.sort();
        for name in animated {
            let Some(rect) = sprites.get(&name).copied() else {
                continue;
            };
            // A registered atlas's sprite is named by its public name under its key.
            let registered = name.strip_prefix(REGISTERED_KEY);
            let public = registered.unwrap_or(&name);
            // Another namespace's sprite is named with it (`lobby:item/...`).
            let id = if public.contains(':') {
                public.to_owned()
            } else {
                format!("minecraft:{public}")
            };
            // A GUI sprite animates the file it was made from, an alias's source included; a
            // registered atlas's sprite its staged file, never a model's origin of the name; a
            // block or item sprite is as its atlas finishes it; the other atlases' are files.
            let Ok(layer) = (match (gui_files.get(&name), registered) {
                (Some(file), _) => loaded.insert_file(file),
                (None, Some(_)) => {
                    loaded.insert_file(&crate::atlas_sources::texture_resource(&id, "textures"))
                }
                (None, None) => loaded.try_insert(&id).or_else(|_| {
                    loaded.insert_file(&crate::atlas_sources::texture_resource(&id, "textures"))
                }),
            }) else {
                continue;
            };
            let Some(animation) = loaded.animations.iter().find(|a| a.layer == layer).cloned()
            else {
                continue;
            };
            let (tw, th) = rect.texels((width, height));
            let image = animation.resized_pixels(animation.frame(0), tw, th);
            let x = (rect.u0 * width as f32).round() as usize;
            let y = (rect.v0 * height as f32).round() as usize;
            let row_bytes = tw as usize * 4;
            for row in 0..th as usize {
                let dst = ((y + row) * width as usize + x) * 4;
                let src = row * row_bytes;
                pixels[dst..dst + row_bytes].copy_from_slice(&image[src..src + row_bytes]);
            }
            sprite_animations.push((rect, animation));
        }
        // Each mapped texture's maps at its size in the atlas, paged.
        let material_maps = if maps.is_empty() {
            Default::default()
        } else {
            let mapped: Vec<crate::paged_maps::SpriteMaps> = maps
                .iter()
                .filter_map(|(name, m)| {
                    let rect = sprites.get(name)?;
                    let (tw, th) = rect.texels((width, height));
                    let x = (rect.u0 * width as f32).round() as u32;
                    let y = (rect.v0 * height as f32).round() as u32;
                    Some(crate::paged_maps::SpriteMaps {
                        rect: [x, y, tw, th],
                        maps: m.each_ref().map(|map| {
                            let (mw, mh, pixels) = map.as_ref()?;
                            crate::paged_maps::fit(pixels, (*mw, *mh), (tw, th))
                        }),
                    })
                })
                .collect();
            crate::paged_maps::build((width, height), &mapped, MAX_ATLAS_SIZE)
        };
        let white = sprites["sunrise/white"];
        let glint = Self::glint_texture(root, defaults, "enchanted_glint_item");
        let glint_armor = Self::glint_texture(root, defaults, "enchanted_glint_armor");
        let font = crate::font::Font::load_over(
            root,
            defaults,
            &sprites,
            &pixels,
            (width, height),
            (glyph_region, height - glyph_region),
            white,
        );
        // The GUI atlas's sprites packed apart because a model has the name are found here only.
        let gui: HashMap<String, GuiSprite> = gui_names
            .into_iter()
            .map(|(id, published)| {
                let sprite = match published {
                    GuiPublished::Packed(key, scaling) => {
                        sprites
                            .get(&key)
                            .map_or(GuiSprite::Missing, |rect| GuiSprite::Present {
                                rect: *rect,
                                scaling,
                            })
                    }
                    GuiPublished::Missing => GuiSprite::Missing,
                };
                (id, sprite)
            })
            .collect();
        let registered: HashMap<String, Rect> = sprites
            .iter()
            .filter_map(|(key, rect)| Some((key.strip_prefix(REGISTERED_KEY)?.to_owned(), *rect)))
            .collect();
        sprites.retain(|name, _| !name.starts_with(GUI_KEY) && !name.starts_with(REGISTERED_KEY));
        // A registered sprite no packed sprite shares its name with keeps its sampling flags under
        // the public name, where a raw texture lookup finds both.
        for name in registered
            .keys()
            .filter(|name| !sprites.contains_key(*name))
        {
            if let Some(value) = flags.remove(&format!("{REGISTERED_KEY}{name}")) {
                flags.insert(name.clone(), value);
            }
        }
        Ok(Self {
            pictures: std::sync::Mutex::new(pictures::Cache::new(root, defaults)),
            revision: Self::next_revision(),
            material_maps: std::sync::Arc::new(material_maps),
            galactic_widths: galactic_widths(&sprites, &pixels, width, height),
            pixels,
            packed: None,
            width,
            height,
            sprites,
            gui,
            non_gui,
            registered,
            live: HashMap::new(),
            object_skins: HashMap::new(),
            object_skins_revision: Self::next_revision(),
            translucent,
            flags,
            sprite_animations,
            glint,
            glint_armor,
            text_shaders: Default::default(),
            lines_shader: None,
            lines_expressions: Vec::new(),
            sky_shader: None,
            stars_shader: None,
            sky_expressions: Vec::new(),
            stars_expressions: Vec::new(),
            lightmap_shader: None,
            lightmap_expressions: Vec::new(),
            clouds_shader: None,
            sprite_modifiers: Default::default(),
            random_textures,
            emissive,
            blinks,
            font,
            white,
            dynamic_texture_pool: None,
        })
    }
    /// `misc/<name>.png` on its own, or a transparent texel if the resource
    /// pack lacks it.
    fn glint_texture(
        root: &ResourceSource,
        defaults: &ResourceSource,
        name: &str,
    ) -> (Vec<u8>, u32, u32) {
        let relative = format!("assets/minecraft/textures/misc/{name}.png");
        let source = if root.contains(&relative) {
            root
        } else {
            defaults
        };
        decode_resource(source, &relative)
            .map_or((vec![0; 4], 1, 1), |(w, h, pixels)| (pixels, w, h))
    }
}
