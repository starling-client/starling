use super::*;
#[path = "atlas_pack/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;

impl HudAtlas {
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
        let dims: Vec<(u32, u32)> = images.iter().map(|i| (i.1, i.2)).collect();
        Self::layout_dims(&dims, max)
    }
    fn layout_dims(dims: &[(u32, u32)], max: u32) -> Result<(u32, Vec<(u32, u32)>, u32)> {
        let mut width = 4096.min(max);
        loop {
            let wide_enough = dims.iter().all(|&(w, _)| w < width);
            if wide_enough {
                for first_fit in [false, true] {
                    let (placements, glyph_region) =
                        Self::shelves_dims(dims, width, first_fit, true);
                    if glyph_region.saturating_add(GLYPH_ROWS) <= max {
                        return Ok((width, placements, glyph_region));
                    }
                }
            }
            if width >= max {
                let texels: u64 = dims.iter().map(|&(w, h)| u64::from(w) * u64::from(h)).sum();
                return Err(format!(
                    "the textures ({} images, {texels} texels) do not fit a {max}x{max} atlas",
                    dims.len()
                )
                .into());
            }
            width = width.saturating_mul(2).min(max);
        }
    }
    /// `layout`'s shelves at one width: the placements and the first row below
    /// them. `first_fit` puts an image in the first shelf that is tall enough
    /// and has room, instead of only the last one.
    fn shelves_dims(
        dims: &[(u32, u32)],
        width: u32,
        first_fit: bool,
        hud: bool,
    ) -> (Vec<(u32, u32)>, u32) {
        // Each shelf: its top row, its height (its first image's) and the next free column.
        let mut shelves: Vec<(u32, u32, u32)> = Vec::new();
        let mut placements = Vec::with_capacity(dims.len());
        for &(w, h) in dims {
            let fits = |s: &(u32, u32, u32)| {
                s.1 >= h
                    && s.2
                        .checked_add(w)
                        .is_some_and(|end| if hud { end < width } else { end <= width })
            };
            let open = if first_fit {
                shelves.iter().position(fits)
            } else {
                shelves.len().checked_sub(1).filter(|&i| fits(&shelves[i]))
            };
            let i = open.unwrap_or_else(|| {
                let top = shelves.last().map_or(0, |s| {
                    s.0.saturating_add(s.1).saturating_add(u32::from(hud))
                });
                shelves.push((top, h, 0));
                shelves.len() - 1
            });
            placements.push((shelves[i].2, shelves[i].0));
            shelves[i].2 = shelves[i]
                .2
                .saturating_add(w)
                .saturating_add(u32::from(hud));
        }
        let glyph_region = shelves.last().map_or(1, |s| {
            s.0.saturating_add(s.1).saturating_add(u32::from(hud))
        });
        (placements, glyph_region)
    }
    /// Pack the textures under these folders of `textures/`, and for the
    /// `full` atlas the generated sprites and the run-time slots too.
    pub(super) fn pack(
        root: &ResourceSource,
        defaults: &ResourceSource,
        folders: &[&str],
        full: bool,
        max_atlas_size: u32,
    ) -> Result<Self> {
        // Starling: the side the renderer found the GPU takes (`atlas_limit`, which a phone waits
        // for), where the loader started before the device opened.
        let max_atlas_size = max_atlas_size.min(atlas_limit());
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
            // Mod-registered atlases (`ns:gui/atlases/...`) bypass the GUI widget size filter;
            // the renderer's actual texture limit is enforced per registered atlas during layout.
            let registered_atlas = name
                .split_once(':')
                .map_or(name.as_str(), |(_, p)| p)
                .starts_with("gui/atlases/");
            // Vanilla particle atlases accept sprites of any size.
            let particle = name.starts_with("particle/") || name.contains(":particle/");
            let (gw, gh) = if particle || registered_atlas {
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
        images.push((MISSING_GLYPH_SPRITE.into(), 5, 8, missing_glyph_rgba()));
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
            Self::slots(&mut images, crate::trims::slot_size(root));
            let (width, height, missing) = missing_image();
            images.push((MISSING_SPRITE.into(), width, height, missing));
            let (width, height, rgba) =
                decode_bytes(include_bytes!("../jei_recipes/config_button.png"))?;
            images.push(("sunrise/recipes/settings".to_owned(), width, height, rgba));
        }
        let mut base_images = Vec::with_capacity(images.len());
        let mut registered_images = Vec::new();
        for image in images {
            if image.0.starts_with(REGISTERED_KEY) {
                registered_images.push(image);
            } else {
                base_images.push(image);
            }
        }
        // Starling: on a small GPU (a phone's) the static sprites shrink to fit it before their
        // borders are wrapped (`fit_mobile_atlas`); each keeps the size it draws at (`full_sizes`).
        let mut full_sizes = HashMap::new();
        #[cfg(target_os = "android")]
        if max_atlas_size < MAX_ATLAS_SIZE {
            Self::fit_mobile_atlas(&mut base_images, &mut full_sizes, &flags, max_atlas_size)?;
        }
        let (blur_bordered, clamp_bordered, sprite_borders) =
            pad_sprite_borders(&mut base_images, &flags, u32::MAX)?;
        base_images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        // Starling: what still doesn't fit goes to half size, the largest first (`layout_within`),
        // before the models are reduced. A sprite with a border keeps its size.
        let padded: HashSet<String> = blur_bordered.union(&clamp_bordered).cloned().collect();
        let (width, placements, glyph_region) = Self::layout_within(
            &mut base_images,
            &mut animated,
            &cit_names,
            &padded,
            max_atlas_size,
            &mut full_sizes,
        )
                .or_else(|error| -> Result<_> {
                let models = reducible_model_sprites(root, &base_images, &blur_bordered);
                let layout = Self::layout_reducing_models_with_borders(
                    &mut base_images,
                    &mut gui_sizes,
                    &models,
                    &clamp_bordered,
                    max_atlas_size,
                )?;
                log::warn!(
                    "reduced HUD model texture resolution to keep the resource pack: {error}"
                );
                Ok(layout)
            })?;
        // A multiple of 1024 rather than a power of two: the default resources
        // need 4096x5120 (8192 rows before), Faithful 32x 4096x12288 (16384).
        let height = glyph_region.saturating_add(GLYPH_ROWS).saturating_add(1023) / 1024 * 1024;
        let height = height.min(max_atlas_size);
        let size = [width, height];
        // Starling: a sprite shrunk to fit a small GPU keeps the size it draws at.
        for (name, size) in full_sizes {
            gui_sizes.entry(name).or_insert(size);
        }
        let Stitched {
            mut pixels,
            mut sprites,
            placed_sprites,
            translucent,
        } = stitch_pixels(base_images, placements, size, &gui_sizes, &sprite_borders)?;
        let mut loaded = crate::sprites::Sprites::new(root.clone());
        animated.sort();
        let (sprite_animations, sprite_animation_placements) = initialize_animations(
            &mut loaded,
            &animated,
            &sprites,
            &placed_sprites,
            &gui_files,
            &mut pixels,
            size,
        )?;
        let (registered, registered_owners, registered_pages) = stitch_registered(
            registered_images,
            root,
            max_atlas_size,
            &gui_sizes,
            &flags,
            &animated,
            &mut loaded,
        );
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
            crate::paged_maps::build((width, height), &mapped, max_atlas_size)
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
        sprites.retain(|name, _| !name.starts_with(GUI_KEY));
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
        let skin_origins = (0..SKIN_SLOTS)
            .map_while(|i| placed_sprites.get(&format!("sunrise/skin{i}")))
            .map(|p| [p.x, p.y])
            .collect();
        Ok(Self {
            skin_origins,
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
            registered_owners,
            registered_pages,
            stitch_limit: Some(max_atlas_size),
            object_skins: HashMap::new(),
            object_skins_revision: Self::next_revision(),
            translucent,
            flags,
            sprite_animations,
            sprite_animation_placements,
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

/// Size of a tightly packed RGBA8 image. Reject a size outside Vec's address space
/// before attempting to reserve it (TextureAtlas.createTexture owns one texture per atlas).
pub(crate) fn rgba_len([width, height]: [u32; 2]) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
        .filter(|&len| len <= isize::MAX as usize)
}

fn blit_rgba(
    pixels: &mut [u8],
    [width, height]: [u32; 2],
    [x, y]: [u32; 2],
    [w, h]: [u32; 2],
    source: &[u8],
) -> Result<()> {
    let invalid = || "sprite blit is outside its atlas or its RGBA data";
    if x.checked_add(w).is_none_or(|end| end > width)
        || y.checked_add(h).is_none_or(|end| end > height)
        || rgba_len([w, h]) != Some(source.len())
        || rgba_len([width, height]) != Some(pixels.len())
    {
        return Err(invalid().into());
    }
    if w == 0 || h == 0 {
        return Ok(());
    }
    let stride = rgba_len([width, 1]).ok_or_else(invalid)?;
    let offset = rgba_len([x, 1]).ok_or_else(invalid)?;
    let end = offset
        .checked_add(rgba_len([w, 1]).ok_or_else(invalid)?)
        .ok_or_else(invalid)?;
    for (dst, src) in pixels
        .chunks_exact_mut(stride)
        .skip(y as usize)
        .zip(source.chunks_exact(end - offset))
    {
        dst[offset..end].copy_from_slice(src);
    }
    Ok(())
}

// Blurred sprites wrap their opposite edges so linear filtering repeats cleanly;
// every other non-font packed sprite gets a 1-texel clamp-to-edge border matching
// vanilla 26.3's `Stitcher` padding + `ANIMATE_SPRITE_BLIT` `CLAMP_TO_EDGE` so
// boundary samples never land on the transparent atlas gutter and discard.
fn should_clamp_pad(name: &str) -> bool {
    if name == "sunrise/white" || name == MISSING_GLYPH_SPRITE {
        return false;
    }
    let bare = name
        .strip_prefix(GUI_KEY)
        .or_else(|| name.strip_prefix(REGISTERED_KEY))
        .unwrap_or(name);
    let path = bare.split_once(':').map_or(bare, |(_, p)| p);
    !path.starts_with("font/")
}

/// Each sprite gets a one-texel border by its flags, except one whose padded size would pass
/// `limit` (the device's texture size): it is packed at its own size, as the sprite that exactly
/// fills the device must be, and sampled at the page edge without texels beyond it.
fn pad_sprite_borders(
    images: &mut [PackedImage],
    flags: &HashMap<String, TextureFlags>,
    limit: u32,
) -> Result<(
    HashSet<String>,
    HashSet<String>,
    HashMap<String, AtlasBorder>,
)> {
    let mut blur_bordered = HashSet::new();
    let mut clamp_bordered = HashSet::new();
    let mut sprite_borders = HashMap::new();
    for image in images {
        if image.3.is_empty() || image.1 == 0 || image.2 == 0 {
            continue;
        }
        if image.1.saturating_add(2) > limit || image.2.saturating_add(2) > limit {
            continue;
        }
        let f = flags.get(&image.0).copied();
        if f.is_some_and(|f| f.blur && !f.clamp) {
            *image = wrap_border(image)?;
            blur_bordered.insert(image.0.clone());
            sprite_borders.insert(image.0.clone(), AtlasBorder::Wrap);
        } else if f.is_some_and(|f| f.blur) || should_clamp_pad(&image.0) {
            *image = clamp_border(image)?;
            clamp_bordered.insert(image.0.clone());
            sprite_borders.insert(image.0.clone(), AtlasBorder::Clamp);
        }
    }
    Ok((blur_bordered, clamp_bordered, sprite_borders))
}

struct Stitched {
    pixels: Vec<u8>,
    sprites: HashMap<String, Rect>,
    placed_sprites: HashMap<String, AnimatedPlacement>,
    translucent: HashSet<String>,
}

fn stitch_pixels(
    images: Vec<PackedImage>,
    placements: Vec<(u32, u32)>,
    size: [u32; 2],
    gui_sizes: &HashMap<String, (u32, u32)>,
    sprite_borders: &HashMap<String, AtlasBorder>,
) -> Result<Stitched> {
    let [width, height] = size;
    let count = rgba_len(size).ok_or("RGBA8 atlas size exceeds the address space")?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count)?;
    pixels.resize(count, 0);
    let mut sprites = HashMap::new();
    let mut placed_sprites = HashMap::new();
    let mut translucent = HashSet::new();
    for ((name, w, h, data), (px, py)) in images.into_iter().zip(placements) {
        if data
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] != 0 && p[3] != 255)
        {
            translucent.insert(name.clone());
        }
        if !data.is_empty() {
            blit_rgba(&mut pixels, size, [px, py], [w, h], &data)?;
        }
        let border = sprite_borders
            .get(&name)
            .copied()
            .unwrap_or(AtlasBorder::None);
        let b = u32::from(border != AtlasBorder::None);
        let (px, py, w, h) = (px + b, py + b, w - 2 * b, h - 2 * b);
        placed_sprites.insert(
            name.clone(),
            AnimatedPlacement {
                x: px,
                y: py,
                w,
                h,
                border,
            },
        );
        let (gw, gh) = gui_sizes.get(&name).copied().unwrap_or((w, h));
        sprites.insert(
            name,
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
    Ok(Stitched {
        pixels,
        sprites,
        placed_sprites,
        translucent,
    })
}

/// Initialize each animated sprite with its first scheduled frame, not strip frame zero.
fn initialize_animations(
    loaded: &mut crate::sprites::Sprites,
    names: &[String],
    sprites: &HashMap<String, Rect>,
    placed_sprites: &HashMap<String, AnimatedPlacement>,
    files: &HashMap<String, String>,
    pixels: &mut [u8],
    [width, height]: [u32; 2],
) -> Result<(
    Vec<(Rect, crate::sprites::SpriteAnimation)>,
    Vec<AnimatedPlacement>,
)> {
    let mut animations = Vec::new();
    let mut placements = Vec::new();
    for name in names {
        let Some(&rect) = sprites.get(name) else {
            continue;
        };
        let Some(&placement) = placed_sprites.get(name) else {
            continue;
        };
        let registered = name.strip_prefix(REGISTERED_KEY);
        let public = registered.unwrap_or(name);
        let id = if public.contains(':') {
            public.to_owned()
        } else {
            format!("minecraft:{public}")
        };
        let texture = || crate::atlas_sources::texture_resource(&id, "textures");
        let Ok(layer) = (match (files.get(name), registered) {
            (Some(file), _) => loaded.insert_file(file),
            (None, Some(_)) => loaded.insert_file(&texture()),
            (None, None) => loaded
                .try_insert(&id)
                .or_else(|_| loaded.insert_file(&texture())),
        }) else {
            continue;
        };
        let Some(animation) = loaded.animations.iter().find(|a| a.layer == layer).cloned() else {
            continue;
        };
        let (w, h) = (placement.w, placement.h);
        let image = animation.resized_pixels(animation.frame(0), w, h)?;
        blit_rgba(
            pixels,
            [width, height],
            [placement.x, placement.y],
            [w, h],
            &image,
        )?;
        match placement.border {
            AtlasBorder::None => {}
            AtlasBorder::Clamp => {
                write_atlas_border(pixels, width, placement.x, placement.y, w, h, false);
            }
            AtlasBorder::Wrap => {
                write_atlas_border(pixels, width, placement.x, placement.y, w, h, true);
            }
        }
        animations.push((rect, animation));
        placements.push(placement);
    }
    Ok((animations, placements))
}

// Like SpriteLoader.stitch, one registered atlas gets its own layout and the entire
// opened-device limit, without the HUD's reserved glyph rows or other atlases' sprites.
fn registered_layout(images: &[PackedImage], max: u32) -> Result<([u32; 2], Vec<(u32, u32)>)> {
    let dims: Vec<_> = images.iter().map(|i| (i.1, i.2)).collect();
    let widest = dims.iter().map(|d| d.0).max().unwrap_or(1);
    if widest <= max {
        let mut width = widest.checked_next_power_of_two().unwrap_or(max).min(max);
        loop {
            for first_fit in [false, true] {
                let (placements, height) = HudAtlas::shelves_dims(&dims, width, first_fit, false);
                if height <= max {
                    return Ok(([width, height], placements));
                }
            }
            if width == max {
                break;
            }
            width = width.saturating_mul(2).min(max);
        }
    }
    Err(format!("{} sprites do not fit a {max}x{max} atlas", images.len()).into())
}

fn stitch_registered(
    images: Vec<PackedImage>,
    root: &ResourceSource,
    max: u32,
    gui_sizes: &HashMap<String, (u32, u32)>,
    flags: &HashMap<String, TextureFlags>,
    animated: &[String],
    loaded: &mut crate::sprites::Sprites,
) -> (
    HashMap<String, Rect>,
    HashMap<String, usize>,
    Vec<RegisteredAtlasPage>,
) {
    let staged = root.cached_get::<crate::atlas_sources::StagedTextureAtlases>();
    let mut groups = std::collections::BTreeMap::<(String, Option<&'static str>), Vec<_>>::new();
    for image in images {
        let public = image
            .0
            .strip_prefix(REGISTERED_KEY)
            .expect("registered image");
        let canonical = crate::atlas_sources::qualified(public);
        let owner = staged
            .as_ref()
            .and_then(|s| {
                let &index = s.sprite_atlas.get(canonical.as_str())?;
                let (atlas, mod_id) = &s.order[index];
                Some((atlas.clone(), Some(*mod_id)))
            })
            .unwrap_or_else(|| {
                let (namespace, path) = canonical.split_once(':').expect("qualified sprite");
                let atlas = path
                    .strip_prefix("gui/atlases/")
                    .and_then(|p| p.split('/').next())
                    .unwrap_or("");
                (format!("{namespace}:{atlas}"), None)
            });
        groups.entry(owner).or_default().push(image);
    }
    let mut registered = HashMap::new();
    let mut owners = HashMap::new();
    let mut pages = Vec::new();
    for ((atlas, mod_id), mut images) in groups {
        let build = || -> Result<_> {
            let (_, _, sprite_borders) = pad_sprite_borders(&mut images, flags, max)?;
            images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
            let (size, placements) = registered_layout(&images, max)?;
            let Stitched {
                mut pixels,
                sprites,
                placed_sprites,
                ..
            } = stitch_pixels(images, placements, size, gui_sizes, &sprite_borders)?;
            let (animations, animation_placements) = initialize_animations(
                loaded,
                animated,
                &sprites,
                &placed_sprites,
                &HashMap::new(),
                &mut pixels,
                size,
            )?;
            Ok((size, pixels, sprites, animations, animation_placements))
        };
        match build() {
            Ok((size, pixels, sprites, animations, animation_placements)) => {
                let page = pages.len();
                for (key, rect) in sprites {
                    let name = key.strip_prefix(REGISTERED_KEY).expect("registered image");
                    registered.insert(name.to_owned(), rect);
                    owners.insert(name.to_owned(), page);
                }
                pages.push(RegisteredAtlasPage {
                    atlas,
                    mod_id,
                    size,
                    pixels,
                    animations,
                    animation_placements,
                });
            }
            Err(error) => log::error!(
                "Sunrise: texture atlas '{atlas}' (mod {mod_id:?}) could not be stitched: {error}"
            ),
        }
    }
    (registered, owners, pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn individually_fitting_mod_atlases_keep_both_pages_when_a_shared_layout_overflows() {
        const FIRST: &str = "demo:gui/atlases/first/demo/a";
        const SECOND: &str = "demo:gui/atlases/second/demo/b";
        let images = [(FIRST, [255, 0, 0, 255]), (SECOND, [0, 0, 255, 255])]
            .map(|(name, color)| (format!("{REGISTERED_KEY}{name}"), 9, 9, color.repeat(81)));
        assert!(registered_layout(&images, 16).is_err());
        let source = ResourceSource::default();
        let mut loaded = crate::sprites::Sprites::new(source.clone());
        let (registered, owners, pages) = stitch_registered(
            Vec::from(images),
            &source,
            16,
            &HashMap::new(),
            &HashMap::new(),
            &[],
            &mut loaded,
        );
        let mut atlas = HudAtlas::boot();
        atlas.registered = registered;
        atlas.registered_owners = owners;
        let (a, b) = (
            atlas.registered_owners[FIRST],
            atlas.registered_owners[SECOND],
        );
        assert_ne!(a, b);
        assert_eq!(&pages[a].pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&pages[b].pixels[..4], &[0, 0, 255, 255]);
        let mut batch = HudBatch::scratch([64, 64]);
        batch.sprite(&atlas, FIRST, 0.0, 0.0);
        batch.sprite(&atlas, SECOND, 0.0, 0.0);
        assert_eq!(
            batch.runs(),
            [
                (0..6, Blend::AtlasPage(a, AtlasBlend::Alpha)),
                (6..12, Blend::AtlasPage(b, AtlasBlend::Alpha))
            ]
        );
    }

    #[test]
    fn rgba_size_and_blits_do_not_wrap_at_large_dimensions() {
        assert_eq!(
            rgba_len([32768, 32768]).map(|n| n as u64),
            if usize::BITS > 32 {
                Some(1u64 << 32)
            } else {
                None
            }
        );
        assert_eq!(rgba_len([u32::MAX, u32::MAX]), None);
        let mut data = [0; 24];
        blit_rgba(&mut data, [3, 2], [1, 1], [2, 1], &[11; 8]).unwrap();
        assert_eq!(&data[16..], &[11; 8]);
        assert!(blit_rgba(&mut data, [3, 2], [2, 1], [2, 1], &[33; 8]).is_err());
        assert_eq!(&data[16..], &[11; 8]);
    }
}
