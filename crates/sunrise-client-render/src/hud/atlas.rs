use super::*;

pub struct HudAtlas {
    pub(super) pictures: std::sync::Mutex<pictures::Cache>,
    /// Identity of this packed atlas, used to invalidate cached model UV data
    /// even when a resource reload places sprites at the same coordinates.
    pub revision: u64,
    /// RGBA8, `width` by `height`, as built: empty once `pack_pixels` has
    /// compressed them (read them through `pixels()`).
    pub pixels: Vec<u8>,
    /// `pixels` compressed (lz4) on a thread of its own once the renderer
    /// has uploaded them: only rebuilding a renderer reads them again.
    pub(super) packed: Option<Arc<OnceLock<Vec<u8>>>>,
    /// The width each Standard Galactic letter uses in `font/ascii_sga`'s
    /// cell, by ASCII code (the enchanting table's text), read when built.
    pub galactic_widths: [u16; 128],
    pub width: u32,
    pub height: u32,
    pub sprites: HashMap<String, Rect>,
    /// The GUI atlas's own sprites by GUI id, each with an entry of its own: its rect, or none for
    /// a sprite its list removed or failed ([`HudAtlas::gui_lookup`]).
    pub gui: HashMap<String, GuiSprite>,
    /// The packed names no GUI lookup of an id the GUI list lacks answers with: the model atlases'
    /// sprites and the walked files a GUI entry stands on.
    pub non_gui: HashSet<String>,
    /// A registered atlas's sprites by their GUI names (`mymod:gui/atlases/icons/mymod/heart`),
    /// each packed under a key of its own so that no model or GUI sprite can take its place.
    pub registered: HashMap<String, Rect>,
    /// The live (dynamic) textures by GUI id, a lane of their own: the GUI lookup reads it first,
    /// and no reload of another lane writes it ([`dynamic_textures::Names`]).
    pub live: HashMap<String, Rect>,
    /// Resolved player-profile names and UUIDs for inline head components.
    pub object_skins: HashMap<String, String>,
    pub object_skins_revision: u64,
    /// Sprites with a 0 < alpha < 255 texel (`SpriteContents.transparency().hasTranslucent`),
    /// which puts their particles on vanilla's translucent layer.
    pub translucent: HashSet<String>,
    /// Sprites whose `.png.mcmeta` sets `blur` or `clamp`; the others
    /// sample as vanilla's defaults (nearest, repeat).
    pub flags: HashMap<String, TextureFlags>,
    /// Every animated block or item sprite the atlas packs: its atlas rect
    /// and the mcmeta-driven frame list, re-uploaded into the rect as the
    /// frames turn (vanilla's stitched atlas animates all of these).
    pub sprite_animations: Vec<(Rect, crate::sprites::SpriteAnimation)>,
    /// The default font (bitmap sheets in this atlas, unifont rasterized on demand).
    pub font: crate::font::Font,
    /// `misc/enchanted_glint_item` as its own image: vanilla's `GlintSampler`
    /// repeats it per pixel, which one sprite inside this atlas cannot.
    pub glint: (Vec<u8>, u32, u32),
    /// `misc/enchanted_glint_armor`, the armour layers' `GlintSampler`.
    pub glint_armor: (Vec<u8>, u32, u32),
    pub text_shaders: crate::gfx::core_text::TextShaders,
    /// A resource pack's own `core/rendertype_lines` shader for the block outline, translated;
    /// none draws the outline natively.
    pub lines_shader: Option<std::sync::Arc<crate::core_shader::CoreProgram>>,
    /// Its Polytone expression uniforms (`shader_modifiers`), resolved when it loaded.
    pub lines_expressions: Vec<crate::polytone_post::ExpressionUniform>,
    /// The packs' `core/sky` and `core/stars`, when a pack replaces them.
    pub sky_shader: Option<std::sync::Arc<crate::core_shader::CoreProgram>>,
    pub stars_shader: Option<std::sync::Arc<crate::core_shader::CoreProgram>>,
    /// Their Polytone expression uniforms, resolved when they loaded.
    pub sky_expressions: Vec<crate::polytone_post::ExpressionUniform>,
    pub stars_expressions: Vec<crate::polytone_post::ExpressionUniform>,
    /// The packs' `core/lightmap` (with `core/screenquad`), when a pack replaces either stage.
    pub lightmap_shader: Option<std::sync::Arc<crate::core_shader::CoreProgram>>,
    /// Its Polytone expression uniforms (`shader_modifiers`), resolved when it loaded: the
    /// blocks its stages read, with their programs.
    pub lightmap_expressions: Vec<crate::polytone_post::ExpressionUniform>,
    /// The packs' `core/clouds`, when a pack replaces either stage: outside and under Improved
    /// Transparency.
    pub clouds_shader: Option<std::sync::Arc<crate::clouds::pack::Programs>>,
    /// The packs' Polytone screen sprite modifiers.
    pub sprite_modifiers: std::sync::Arc<sprite_modifiers::SpriteModifiers>,
    /// The packs' OptiFine random entity textures, whose variants this atlas packs.
    pub random_textures: sunrise_optifine::random_textures::RandomTextures,
    /// Their emissive overlays (`_e`), packed here too.
    pub emissive: std::sync::Arc<sunrise_optifine::emissive::Emissive>,
    /// ETF's blinking frames (`_blink`), packed here too.
    pub blinks: sunrise_optifine::blink::Blinks,
    /// Of the shared atlas's textures, their LabPBR normal and specular maps as paged maps
    /// (`paged_maps.rs`), read while loading for shader packs (`sprites::labpbr_maps`).
    pub material_maps: std::sync::Arc<[Option<crate::paged_maps::PagedMap>; 2]>,
    /// The white texel flat fills sample (also `hud_kit::shapes`).
    pub(crate) white: Rect,
    /// Where the slots of client mods' dynamic textures lie, when the atlas packed them
    /// ([`HudAtlas::build_over_with_dynamic_textures`]); their names are not among `sprites`.
    pub dynamic_texture_pool: Option<dynamic_textures::Pool>,
}
/// Atlas rows reserved for glyphs rasterized at run time.
pub const GLYPH_ROWS: u32 = 512;
/// The largest atlas side: the 2D texture limit of the GPUs Sunrise runs on
/// (Apple silicon, RADV and Intel allow 16384; NVIDIA 32768).
pub const MAX_ATLAS_SIZE: u32 = 16384;
/// Below this a texture at half size stops being worth what it costs to look at.
pub(super) const SMALLEST_SHRUNK_SIDE: u32 = 32;
/// The interface's textures shrink only once the others are down to this side.
pub(super) const INTERFACE_WAITS_FOR_SIDE: u32 = 64;
/// A side of a GPU's largest texture, once the renderer has found the GPU; 0 before.
static GPU_TEXTURE_SIDE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The renderer's word on what the GPU takes, which the atlas is packed within.
pub fn set_gpu_texture_side(side: u32) {
    GPU_TEXTURE_SIDE.store(side, std::sync::atomic::Ordering::Relaxed);
}
/// The most an atlas may measure a side. On a phone the renderer finds the GPU while the atlas
/// loads, and the small GPUs are there, so this waits for its word (not for ever: it only asks
/// the driver); elsewhere a GPU not found yet counts as one that takes the most.
pub(super) fn atlas_limit() -> u32 {
    let side = || GPU_TEXTURE_SIDE.load(std::sync::atomic::Ordering::Relaxed);
    if cfg!(target_os = "android") {
        let start = std::time::Instant::now();
        while side() == 0 && start.elapsed() < std::time::Duration::from_secs(20) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    match side() {
        0 => MAX_ATLAS_SIZE,
        side => side.min(MAX_ATLAS_SIZE),
    }
}
/// `image` at half its size, by every other texel: no new colors or alpha, so what was cut out
/// stays cut out.
pub(super) fn halved((name, w, h, data): &PackedImage) -> PackedImage {
    let (half_w, half_h) = (w.div_ceil(2), h.div_ceil(2));
    let mut half = Vec::with_capacity((half_w * half_h * 4) as usize);
    for y in 0..half_h {
        for x in 0..half_w {
            let at = ((y * 2 * w + x * 2) * 4) as usize;
            half.extend_from_slice(&data[at..at + 4]);
        }
    }
    (name.clone(), half_w, half_h, half)
}
/// 128x128 atlas slots for map textures. A lobby paints its signs as walls of
/// framed maps, so the pool holds a wall; too few slots re-upload every map
/// every frame. 26.3's `MapTextureManager` keeps a texture per map instead.
pub const MAP_SLOTS: usize = 256;
/// The atlas sprite of map slot `slot` (`sunrise/map{slot}`), named once:
/// every map shown asks for its slot's each frame.
pub fn map_slot_sprite(slot: usize) -> Option<&'static str> {
    static NAMES: std::sync::LazyLock<Vec<String>> =
        std::sync::LazyLock::new(|| (0..MAP_SLOTS).map(|i| format!("sunrise/map{i}")).collect());
    NAMES.get(slot).map(String::as_str)
}
/// Atlas slots for downloaded player skins (`skins/<sha1>`), 64x64 each.
/// Vanilla registers a texture per skin. Here the slots are shared
/// (`skins::SkinSlots`): skins drawn in the last frame keep theirs, and a
/// skin past them shows the default skin until a slot frees up.
pub const SKIN_SLOTS: usize = 128;
/// Reused by the visible rows of the classic world selector.
pub const WORLD_ICON_SLOTS: usize = 32;
/// Atlas slots for the pack icons on the pack screen, one per icon drawn
/// (`PackSelectionScreen.loadPackIcon` registers a texture per pack): two lists
/// of 36-pixel rows at GUI scale 2 on a 4K screen still fit.
pub const PACK_ICON_SLOTS: usize = 64;
/// A pack icon slot's side: vanilla's own icons are 128x128.
pub const PACK_ICON_SIZE: u32 = 128;
// Bake the settings glyph into its plate: drawing a translucent glyph over
// a translucent plate changes its effective alpha and darkens it twice.
pub(super) fn android_utility_sprites(images: &[PackedImage]) -> Vec<PackedImage> {
    const GEAR: [&str; 13] = [
        ".....###.....",
        "..##.###.##..",
        ".###########.",
        ".###########.",
        "..###...###..",
        "####.....####",
        "####.....####",
        "####.....####",
        "..###...###..",
        ".###########.",
        ".###########.",
        "..##.###.##..",
        ".....###.....",
    ];
    let mut out = Vec::new();
    for name in ["pause", "inventory", "settings"] {
        for pressed in [false, true] {
            let suffix = if pressed { "_pressed" } else { "" };
            let source = if name == "settings" {
                "custom_blank"
            } else {
                name
            };
            let key = format!("starling:gui/sprites/touch/{source}{suffix}");
            let Some((_, w, h, pixels)) = images.iter().find(|i| i.0 == key).or_else(|| {
                images
                    .iter()
                    .find(|i| i.0 == format!("starling:gui/sprites/touch/{source}"))
            }) else {
                continue;
            };
            let mut pixels = pixels.clone();
            if name == "settings" && *w >= 13 && *h >= 13 {
                let ox = (*w - 13) / 2;
                let oy = (*h - 13) / 2;
                for (y, row) in GEAR.iter().enumerate() {
                    for (x, pixel) in row.bytes().enumerate() {
                        if pixel == b'#' {
                            let index = (((oy + y as u32) * w + ox + x as u32) * 4) as usize;
                            let ink = if pressed { 230 } else { 0 };
                            pixels[index..index + 3].fill(ink);
                        }
                    }
                }
            }
            if name != "inventory" {
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel[3] = (u16::from(pixel[3]) * 204 / 255) as u8;
                }
            }
            out.push((
                format!("starling:android-controls/{name}{suffix}"),
                *w,
                *h,
                pixels,
            ));
        }
    }
    out
}

/// Atlas slots for armour trim textures made at run time (`trims`), each the
/// size of the largest trim texture.
pub const TRIM_SLOTS: usize = 32;

/// The prefix of the name a GUI atlas sprite is packed under when a model's sprite has its name.
pub(super) const GUI_KEY: &str = "#gui:";
/// The prefix of the name a registered atlas's sprite is packed under.
pub(super) const REGISTERED_KEY: &str = "#registered:";

/// `MissingTextureAtlasSprite`: what a sprite the atlas lacks draws as.
pub const MISSING_SPRITE: &str = "sunrise/missing";

/// Starling's own touch controls (packaging/touch/make.py draws them): a button's sprite and its
/// `_pressed`, the D-pad's cells and the stick's, as `starling:touch/<name>`.
const TOUCH_SPRITES: &[(&str, &[u8])] = &[
    (
        "custom_blank",
        include_bytes!("../../../sunrise-client/assets/touch/custom_blank.png"),
    ),
    (
        "custom_star",
        include_bytes!("../../../sunrise-client/assets/touch/custom_star.png"),
    ),
    (
        "custom_star_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_star_pressed.png"),
    ),
    (
        "custom_heart",
        include_bytes!("../../../sunrise-client/assets/touch/custom_heart.png"),
    ),
    (
        "custom_heart_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_heart_pressed.png"),
    ),
    (
        "custom_shield",
        include_bytes!("../../../sunrise-client/assets/touch/custom_shield.png"),
    ),
    (
        "custom_shield_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_shield_pressed.png"),
    ),
    (
        "custom_pickaxe",
        include_bytes!("../../../sunrise-client/assets/touch/custom_pickaxe.png"),
    ),
    (
        "custom_pickaxe_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_pickaxe_pressed.png"),
    ),
    (
        "custom_potion",
        include_bytes!("../../../sunrise-client/assets/touch/custom_potion.png"),
    ),
    (
        "custom_potion_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_potion_pressed.png"),
    ),
    (
        "custom_apple",
        include_bytes!("../../../sunrise-client/assets/touch/custom_apple.png"),
    ),
    (
        "custom_apple_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_apple_pressed.png"),
    ),
    (
        "custom_home",
        include_bytes!("../../../sunrise-client/assets/touch/custom_home.png"),
    ),
    (
        "custom_home_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_home_pressed.png"),
    ),
    (
        "custom_flag",
        include_bytes!("../../../sunrise-client/assets/touch/custom_flag.png"),
    ),
    (
        "custom_flag_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_flag_pressed.png"),
    ),
    (
        "custom_fire",
        include_bytes!("../../../sunrise-client/assets/touch/custom_fire.png"),
    ),
    (
        "custom_fire_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_fire_pressed.png"),
    ),
    (
        "custom_skull",
        include_bytes!("../../../sunrise-client/assets/touch/custom_skull.png"),
    ),
    (
        "custom_skull_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_skull_pressed.png"),
    ),
    (
        "custom_crown",
        include_bytes!("../../../sunrise-client/assets/touch/custom_crown.png"),
    ),
    (
        "custom_crown_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_crown_pressed.png"),
    ),
    (
        "custom_plus",
        include_bytes!("../../../sunrise-client/assets/touch/custom_plus.png"),
    ),
    (
        "custom_plus_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_plus_pressed.png"),
    ),
    (
        "custom_check",
        include_bytes!("../../../sunrise-client/assets/touch/custom_check.png"),
    ),
    (
        "custom_check_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_check_pressed.png"),
    ),
    (
        "custom_cross",
        include_bytes!("../../../sunrise-client/assets/touch/custom_cross.png"),
    ),
    (
        "custom_cross_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_cross_pressed.png"),
    ),
    (
        "custom_music",
        include_bytes!("../../../sunrise-client/assets/touch/custom_music.png"),
    ),
    (
        "custom_music_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/custom_music_pressed.png"),
    ),
    (
        "attack",
        include_bytes!("../../../sunrise-client/assets/touch/attack.png"),
    ),
    (
        "attack_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/attack_pressed.png"),
    ),
    (
        "camera",
        include_bytes!("../../../sunrise-client/assets/touch/camera.png"),
    ),
    (
        "camera_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/camera_pressed.png"),
    ),
    (
        "chat",
        include_bytes!("../../../sunrise-client/assets/touch/chat.png"),
    ),
    (
        "chat_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/chat_pressed.png"),
    ),
    (
        "command",
        include_bytes!("../../../sunrise-client/assets/touch/command.png"),
    ),
    (
        "command_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/command_pressed.png"),
    ),
    (
        "debug",
        include_bytes!("../../../sunrise-client/assets/touch/debug.png"),
    ),
    (
        "debug_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/debug_pressed.png"),
    ),
    (
        "dpad_down",
        include_bytes!("../../../sunrise-client/assets/touch/dpad_down.png"),
    ),
    (
        "dpad_down_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/dpad_down_pressed.png"),
    ),
    (
        "dpad_left",
        include_bytes!("../../../sunrise-client/assets/touch/dpad_left.png"),
    ),
    (
        "dpad_left_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/dpad_left_pressed.png"),
    ),
    (
        "dpad_middle",
        include_bytes!("../../../sunrise-client/assets/touch/dpad_middle.png"),
    ),
    (
        "dpad_right",
        include_bytes!("../../../sunrise-client/assets/touch/dpad_right.png"),
    ),
    (
        "dpad_right_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/dpad_right_pressed.png"),
    ),
    (
        "dpad_up",
        include_bytes!("../../../sunrise-client/assets/touch/dpad_up.png"),
    ),
    (
        "dpad_up_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/dpad_up_pressed.png"),
    ),
    (
        "drop",
        include_bytes!("../../../sunrise-client/assets/touch/drop.png"),
    ),
    (
        "drop_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/drop_pressed.png"),
    ),
    (
        "inventory",
        include_bytes!("../../../sunrise-client/assets/touch/inventory.png"),
    ),
    (
        "inventory_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/inventory_pressed.png"),
    ),
    (
        "jump",
        include_bytes!("../../../sunrise-client/assets/touch/jump.png"),
    ),
    (
        "jump_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/jump_pressed.png"),
    ),
    (
        "offhand",
        include_bytes!("../../../sunrise-client/assets/touch/offhand.png"),
    ),
    (
        "offhand_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/offhand_pressed.png"),
    ),
    (
        "pause",
        include_bytes!("../../../sunrise-client/assets/touch/pause.png"),
    ),
    (
        "pause_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/pause_pressed.png"),
    ),
    (
        "pick",
        include_bytes!("../../../sunrise-client/assets/touch/pick.png"),
    ),
    (
        "pick_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/pick_pressed.png"),
    ),
    (
        "players",
        include_bytes!("../../../sunrise-client/assets/touch/players.png"),
    ),
    (
        "players_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/players_pressed.png"),
    ),
    (
        "sneak",
        include_bytes!("../../../sunrise-client/assets/touch/sneak.png"),
    ),
    (
        "sneak_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/sneak_pressed.png"),
    ),
    (
        "stick",
        include_bytes!("../../../sunrise-client/assets/touch/stick.png"),
    ),
    (
        "stick_knob",
        include_bytes!("../../../sunrise-client/assets/touch/stick_knob.png"),
    ),
    (
        "use",
        include_bytes!("../../../sunrise-client/assets/touch/use.png"),
    ),
    (
        "use_pressed",
        include_bytes!("../../../sunrise-client/assets/touch/use_pressed.png"),
    ),
];

/// The names of [`TOUCH_SPRITES`]: every sprite the touch controls draw.
pub fn touch_sprite_names() -> impl Iterator<Item = &'static str> {
    TOUCH_SPRITES.iter().map(|(name, _)| *name)
}

/// [`TOUCH_SPRITES`] a pack has not drawn already (a pack's `starling:touch/jump` wins).
pub(super) fn builtin_touch_sprites(images: &[PackedImage]) -> Vec<PackedImage> {
    TOUCH_SPRITES
        .iter()
        .filter_map(|(name, png)| {
            let key = format!("starling:gui/sprites/touch/{name}");
            if images.iter().any(|(k, ..)| *k == key) {
                return None;
            }
            let (w, h, mut pixels) = decode_bytes(png).ok()?;
            if matches!(*name, "pause" | "inventory") {
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    if pixel[0] >= 245 && pixel[1] >= 245 && pixel[2] >= 245 {
                        pixel[..3].copy_from_slice(&[32, 32, 32]);
                    }
                }
            }
            Some((key, w, h, pixels))
        })
        // A slot for each custom button's own images, whichever it has now.
        .chain(
            (0..crate::own_buttons::pressed_slot(crate::own_buttons::SLOTS)).map(|id| {
                let side = crate::own_buttons::SIZE;
                let blank = || vec![0; (side * side * 4) as usize];
                let pixels = crate::own_buttons::load(id).unwrap_or_else(blank);
                (crate::own_buttons::sprite(id), side, side, pixels)
            }),
        )
        .collect()
}

/// One of [`TOUCH_SPRITES`], as RGBA pixels: a place for a custom button's image to start from.
pub fn touch_sprite(name: &str) -> Option<Vec<u8>> {
    let (_, png) = TOUCH_SPRITES.iter().find(|(n, _)| *n == name)?;
    decode_bytes(png).ok().map(|(_, _, pixels)| pixels)
}

/// The used width of each Standard Galactic letter (A-Z, a-z) in the
/// `font/ascii_sga` sheet: one past its last column with a texel that isn't
/// transparent.
pub(super) fn galactic_widths(
    sprites: &HashMap<String, Rect>,
    pixels: &[u8],
    width: u32,
    height: u32,
) -> [u16; 128] {
    let mut widths = [0u16; 128];
    let Some(sheet) = sprites.get("font/ascii_sga") else {
        return widths;
    };
    let cell = sheet.w / 16;
    let (px, py) = (
        (sheet.u0 * width as f32) as u32,
        (sheet.v0 * height as f32) as u32,
    );
    for code in (0x41..=0x5Au32).chain(0x61..=0x7A) {
        let (cx, cy) = (px + code % 16 * cell, py + code / 16 * cell);
        let alpha = |x: u32, y: u32| {
            pixels
                .get(((y * width + x) * 4 + 3) as usize)
                .copied()
                .unwrap_or(0)
        };
        widths[code as usize] = (0..cell)
            .rev()
            .find(|&dx| (0..cell).any(|dy| alpha(cx + dx, cy + dy) != 0))
            .map_or(0, |dx| (dx + 1).min(u32::from(u16::MAX)) as u16);
    }
    widths
}
/// `TextureAtlas.dumpContents` (`F3 + S`, vanilla's "dynamic textures"): the atlas
/// written as `name.png` beside `name.txt`, every sprite in name order with its
/// rect in texels (`dumpSpriteNames`). Vanilla names each file after its atlas
/// with `/` and `:` as `_` (`Identifier.toDebugFileName`); Sunrise packs the GUI,
/// font, particle, map and skin sprites into one atlas, so one file stands in for
/// the atlases vanilla dumps apart.
pub fn dump_atlas(
    dir: &Path,
    name: &str,
    width: u32,
    height: u32,
    pixels: &[u8],
    sprites: &HashMap<String, Rect>,
) -> std::io::Result<PathBuf> {
    // A build whose pixels were freed (`fonts`) has the size and no texels:
    // write nothing rather than a file every reader would reject.
    if pixels.len() != (width as usize) * (height as usize) * 4 {
        return Err(std::io::Error::other(format!(
            "wrong data size, expected {} got {}",
            (width as usize) * (height as usize) * 4,
            pixels.len()
        )));
    }
    fs::create_dir_all(dir)?;
    let png_path = dir.join(format!("{name}.png"));
    let mut encoder = png::Encoder::new(fs::File::create(&png_path)?, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer
        .write_image_data(pixels)
        .map_err(std::io::Error::other)?;
    drop(writer);
    let mut names = sprites.iter().collect::<Vec<_>>();
    names.sort_by(|a, b| a.0.cmp(b.0));
    let mut list = String::new();
    for (sprite, rect) in names {
        let (w, h) = rect.texels((width, height));
        let x = (rect.u0 * width as f32).round() as u32;
        let y = (rect.v0 * height as f32).round() as u32;
        list.push_str(&format!("{sprite}\tx={x}\ty={y}\tw={w}\th={h}\n"));
    }
    fs::write(dir.join(format!("{name}.txt")), list)?;
    Ok(png_path)
}

impl HudAtlas {
    /// `TextureAtlas.dumpContents` for this atlas: `dir` is `screenshots/debug`,
    /// the folder `TextureUtil.getDebugTexturePath` names.
    pub fn dump_contents(&self, dir: &Path, name: &str) -> std::io::Result<PathBuf> {
        // `pixels()`: the copy `pack_pixels` compressed once the GPU had the
        // atlas, decompressed; vanilla reads the texture back from the GPU.
        let pixels = self.pixels();
        dump_atlas(dir, name, self.width, self.height, &pixels, &self.sprites)
    }

    pub(super) fn next_revision() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
    /// The atlas's pixels: as built, or uncompressed again after
    /// `pack_pixels` (a renderer being rebuilt).
    pub fn pixels(&self) -> std::borrow::Cow<'_, [u8]> {
        let Some(packed) = self.packed.as_ref().filter(|_| self.pixels.is_empty()) else {
            return std::borrow::Cow::Borrowed(&self.pixels);
        };
        let size = (self.width * self.height * 4) as usize;
        std::borrow::Cow::Owned(
            lz4_flex::decompress_size_prepended(packed.wait())
                .ok()
                .filter(|p| p.len() == size)
                .unwrap_or_else(|| vec![0; size]),
        )
    }
    /// Keep the pixels compressed from now on, once a renderer has them: the
    /// window's GPU holds the atlas, and a copy as built only serves a
    /// renderer made again (another window, a lost device, another graphics
    /// API). They are compressed on a thread of their own; `pixels()` waits
    /// for it.
    pub fn pack_pixels(&mut self) {
        if self.pixels.is_empty() || self.packed.is_some() {
            return;
        }
        let raw = Arc::new(std::mem::take(&mut self.pixels));
        let slot = Arc::new(OnceLock::new());
        let (job_raw, job_slot) = (raw.clone(), slot.clone());
        let spawned = std::thread::Builder::new()
            .name("Atlas packing".into())
            .spawn(move || {
                let _ = job_slot.set(lz4_flex::compress_prepend_size(&job_raw));
            });
        if spawned.is_err() {
            let _ = slot.set(lz4_flex::compress_prepend_size(&raw));
        }
        self.packed = Some(slot);
    }
    /// The sprite of the GUI id `id` (`minecraft:` dropped) that is there, with its scaling: a live
    /// texture's; else the GUI list's own (a removed or failed entry ends the search); else, where
    /// the list says nothing of the id, a registered atlas's sprite of the name, and last a packed
    /// sprite no model or GUI entry owns.
    pub(super) fn gui_present(&self, id: &str) -> Option<(&Rect, Option<crate::ui::NineSlice>)> {
        if let Some(rect) = self.live.get(id) {
            return Some((rect, None));
        }
        match self.gui.get(id) {
            Some(GuiSprite::Present { rect, scaling }) => Some((rect, *scaling)),
            Some(GuiSprite::Missing) => None,
            None => self
                .registered
                .get(id)
                .or_else(|| self.sprites.get(id).filter(|_| !self.non_gui.contains(id)))
                .map(|rect| (rect, None)),
        }
    }
    /// A GUI atlas sprite by its id (`minecraft:container/slot`, `container/slot`, a mod's
    /// `mymod:panel`; the two spellings of a vanilla id are one key): [`GuiSprite::Present`] for a
    /// live texture, the GUI list's sprite, or for an unlisted id a registered atlas's sprite or a
    /// packed sprite no model or GUI entry owns, in that order; [`GuiSprite::Missing`] for an id the list removed or failed; none for an id the GUI
    /// atlas does not own. The sprite carries the scaling of the entry itself.
    pub fn gui_lookup(&self, id: &str) -> Option<GuiSprite> {
        let id = id.strip_prefix("minecraft:").unwrap_or(id);
        match self.gui_present(id) {
            Some((rect, scaling)) => Some(GuiSprite::Present {
                rect: *rect,
                scaling,
            }),
            None => self.gui.get(id).copied(),
        }
    }
    /// The rect a GUI draw uses for GUI sprite `id`: [`HudAtlas::gui_lookup`]'s, the missing
    /// sprite for an id the list removed or failed, none for an id the atlas does not own.
    pub fn gui_rect(&self, id: &str) -> Option<&Rect> {
        let id = id.strip_prefix("minecraft:").unwrap_or(id);
        match self.gui_present(id) {
            Some((rect, _)) => Some(rect),
            None => self
                .gui
                .contains_key(id)
                .then(|| self.sprites.get(MISSING_SPRITE))
                .flatten(),
        }
    }
    /// [`HudAtlas::gui_rect`] for a draw: an id the atlas lacks draws the missing texture, as vanilla's atlas does.
    pub fn gui_draw_rect(&self, id: &str) -> Option<&Rect> {
        if let Some(rect) = self.gui_rect(id) {
            return Some(rect);
        }
        // An atlas built with no resources has no GUI list, so nothing to miss against.
        if self.gui.is_empty() {
            return None;
        }
        missing_gui_sprite(id);
        self.sprites.get(MISSING_SPRITE)
    }
    /// The rect of a texture by its exact name: the packed sprite, else a registered atlas's
    /// sprite. The raw lookup of the camera draw; a GUI id goes through [`HudAtlas::gui_lookup`].
    pub fn texture_rect(&self, name: &str) -> Option<&Rect> {
        self.sprites.get(name).or_else(|| self.registered.get(name))
    }
    /// The rect of a `Blit` texture by its name: a live texture's first, else
    /// [`HudAtlas::texture_rect`]. One lookup for the refusal and the draw.
    pub fn blit_rect(&self, texture: &str) -> Option<&Rect> {
        self.live
            .get(texture.strip_prefix("minecraft:").unwrap_or(texture))
            .or_else(|| self.texture_rect(texture))
    }
    /// The pool and the tables a client mod's dynamic texture is named in.
    pub fn dynamic_names(
        &mut self,
    ) -> (Option<&dynamic_textures::Pool>, dynamic_textures::Names<'_>) {
        (
            self.dynamic_texture_pool.as_ref(),
            dynamic_textures::Names {
                sprites: &mut self.sprites,
                live: &mut self.live,
                gui: &mut self.gui,
                registered: &self.registered,
            },
        )
    }
    /// [`HudAtlas::gui_rect`], or the missing sprite where the atlas has no such GUI sprite (an
    /// effect without its icon).
    pub fn gui_sprite_or_missing(&self, id: &str) -> Option<Rect> {
        self.gui_rect(id)
            .or_else(|| self.sprites.get(MISSING_SPRITE))
            .copied()
    }
    /// A sprite, or the missing one when the atlas has no such sprite (an
    /// effect without its icon), as a stitched atlas answers.
    pub fn sprite_or_missing(&self, name: &str) -> Option<Rect> {
        self.sprites
            .get(name)
            .or_else(|| self.sprites.get(MISSING_SPRITE))
            .copied()
    }
    pub fn clear_object_skins(&mut self) {
        self.object_skins.clear();
        self.object_skins_revision = Self::next_revision();
    }
    /// Keep a ready profile skin when another source only has its default.
    pub fn set_object_skin(&mut self, key: String, skin: String) {
        use std::collections::hash_map::Entry;
        if self.object_skins.len() >= 1024 && !self.object_skins.contains_key(&key) {
            self.clear_object_skins();
        }
        match self.object_skins.entry(key) {
            Entry::Occupied(entry)
                if skin.starts_with("entity/player/") && self.sprites.contains_key(entry.get()) => {
            }
            Entry::Occupied(mut entry) => {
                if entry.get() != &skin {
                    entry.insert(skin);
                    self.object_skins_revision = Self::next_revision();
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(skin);
                self.object_skins_revision = Self::next_revision();
            }
        }
    }
    /// A font glyph's atlas cell (8x8 GUI pixels) and advance.
    pub fn glyph(&self, ch: char) -> Option<(Rect, f32)> {
        let g = self.font.glyph(ch);
        g.rect.map(|rect| (rect, g.advance))
    }
    /// A font glyph in the default font: its atlas cell (none for blank
    /// glyphs such as the space), its box and advance.
    pub fn glyph_cell(&self, ch: char) -> crate::font::Glyph {
        self.font.glyph(ch)
    }
    pub fn build(root: &ResourceSource) -> Result<Self> {
        Self::build_over(root, root)
    }
    /// The atlas of a resource root that layers packs over `defaults` (the
    /// game's own resources), whose textures give the sprites' GUI sizes.
    pub fn build_over(root: &ResourceSource, defaults: &ResourceSource) -> Result<Self> {
        Self::build_over_with_dynamic_textures(root, defaults, defaults, false)
    }
    /// The atlas above, with the pool of slots for client mods' dynamic textures
    /// (`dynamic_textures::DYNAMIC_TEXTURE_SLOTS` of each class, 10.5 MiB of texels) only when
    /// `pool` says a client mod is loaded: a game without mods packs no slot
    /// and its atlas does not grow. `game` is the game's own resources, without the packs: a
    /// pack's core shader (text, block outline, sky, stars, lightmap) is one that differs from
    /// the game's, so a `root` given as its own `game` loads none.
    pub fn build_over_with_dynamic_textures(
        root: &ResourceSource,
        defaults: &ResourceSource,
        game: &ResourceSource,
        pool: bool,
    ) -> Result<Self> {
        let mut atlas = Self::pack(root, defaults, WALKED_TEXTURE_FOLDERS, true, pool)?;
        atlas.dynamic_texture_pool =
            dynamic_textures::Pool::take(&mut atlas.sprites, atlas.width, atlas.height);
        pack_text::load_shaders(&mut atlas, root, game)?;
        atlas.sprite_modifiers = std::sync::Arc::new(sprite_modifiers::SpriteModifiers::read(
            root,
            atlas.gui.keys(),
        ));
        Ok(atlas)
    }
    /// The packs' core shaders at `root` over the game's own resources `game`, as
    /// [`Self::build_over_with_dynamic_textures`] loads them, for tests of the pipelines.
    #[cfg(test)]
    pub(crate) fn load_pack_shaders(&mut self, root: &ResourceSource, game: &ResourceSource) {
        pack_text::load_shaders(self, root, game).expect("the packs' required shaders");
    }
    /// Empty slots written at run time: map textures (game/blocks/maps/mod.rs),
    /// client mods' dynamic textures (only with `pool`), downloaded skins, world
    /// and pack icons and the on-screen keyboard's text. Their pixels are already zero in
    /// the destination atlas, so keep the source buffers empty and avoid duplicating the
    /// atlas-sized allocation during packing.
    pub(super) fn slots(images: &mut Vec<PackedImage>, trim: (u32, u32), pool: bool) {
        for i in 0..MAP_SLOTS {
            let name = map_slot_sprite(i).unwrap_or_default();
            images.push((name.into(), 128, 128, Vec::new()));
        }
        // The pool of client mods' dynamic textures, only when a client mod is loaded.
        if pool {
            let classes = mod_api::DYNAMIC_TEXTURE_CLASSES;
            for (class, slots) in dynamic_textures::DYNAMIC_TEXTURE_SLOTS.iter().enumerate() {
                for slot in 0..*slots {
                    let (name, side) = (dynamic_textures::slot_sprite(class, slot), classes[class]);
                    images.push((name, side, side, Vec::new()));
                }
            }
        }
        // Downloaded skins (game/interface/scoreboard/mod.rs).
        for i in 0..SKIN_SLOTS {
            images.push((format!("sunrise/skin{i}"), 64, 64, Vec::new()));
        }
        for i in 0..TRIM_SLOTS {
            images.push((format!("sunrise/trim{i}"), trim.0, trim.1, Vec::new()));
        }
        for i in 0..WORLD_ICON_SLOTS {
            images.push((format!("sunrise/world-icon{i}"), 64, 64, Vec::new()));
        }
        for i in 0..PACK_ICON_SLOTS {
            images.push((
                format!("sunrise/pack-icon{i}"),
                PACK_ICON_SIZE,
                PACK_ICON_SIZE,
                Vec::new(),
            ));
        }
        // The on-screen keyboard's text and corner masks (ui_theme), rasterized at run time.
        images.push(("sunrise/ui-text".into(), 2048, 1024, Vec::new()));
    }
    /// Only the font's metrics, for a game that draws nothing (the
    /// integrated or dedicated server): text measures as with the full atlas,
    /// without packing textures or rasterizing glyphs.
    pub fn fonts(root: &ResourceSource) -> Result<Self> {
        let white = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        };
        Ok(Self {
            pictures: Default::default(),
            revision: Self::next_revision(),
            pixels: Vec::new(),
            packed: None,
            material_maps: Default::default(),
            galactic_widths: [0; 128],
            width: 1,
            height: 1,
            sprites: HashMap::new(),
            gui: HashMap::new(),
            non_gui: HashSet::new(),
            registered: HashMap::new(),
            live: HashMap::new(),
            object_skins: HashMap::new(),
            object_skins_revision: Self::next_revision(),
            translucent: HashSet::new(),
            flags: HashMap::new(),
            sprite_animations: Vec::new(),
            font: crate::font::Font::metrics(root),
            glint: (Vec::new(), 1, 1),
            glint_armor: (Vec::new(), 1, 1),
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
            random_textures: Default::default(),
            emissive: Arc::default(),
            blinks: Default::default(),
            white,
            dynamic_texture_pool: None,
        })
    }
    /// The boot screen's atlas, before any pack loads: the theme's run-time
    /// glyphs (`sunrise/ui-text`), a white sprite and the glyph rows over
    /// transparent black. Nothing from a pack: the boot screen draws with
    /// the theme's bundled faces, so the font here has no sheets to read.
    pub fn boot() -> Self {
        let (width, height) = (1024, 1024);
        let text_rows = 512;
        let glyph_top = text_rows + 4;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        // The white sprite the fills and the font's underline pass sample.
        for row in 0..4 {
            let at = ((text_rows + row) * width * 4) as usize;
            pixels[at..at + 16].fill(255);
        }
        let rect = |u0: u32, v0: u32, u1: u32, v1: u32, w: u32, h: u32| Rect {
            u0: u0 as f32 / width as f32,
            v0: v0 as f32 / height as f32,
            u1: u1 as f32 / width as f32,
            v1: v1 as f32 / height as f32,
            w,
            h,
        };
        let mut sprites = HashMap::new();
        sprites.insert(
            "sunrise/white".into(),
            rect(0, text_rows, 4, glyph_top, 4, 4),
        );
        sprites.insert(
            "sunrise/ui-text".into(),
            rect(0, 0, width, text_rows, width, text_rows),
        );
        let white = sprites["sunrise/white"];
        Self {
            pictures: Default::default(),
            revision: Self::next_revision(),
            font: crate::font::Font::load(
                &crate::resource_source::ResourceSource::default(),
                &sprites,
                &pixels,
                (width, height),
                (glyph_top, height - glyph_top),
                white,
            ),
            material_maps: Default::default(),
            galactic_widths: galactic_widths(&sprites, &pixels, width, height),
            pixels,
            packed: None,
            width,
            height,
            sprites,
            translucent: HashSet::new(),
            flags: HashMap::new(),
            gui: HashMap::new(),
            non_gui: HashSet::new(),
            registered: HashMap::new(),
            live: HashMap::new(),
            object_skins: HashMap::new(),
            object_skins_revision: Self::next_revision(),
            sprite_animations: Vec::new(),
            glint: (vec![0; 4], 1, 1),
            glint_armor: (vec![0; 4], 1, 1),
            white,
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
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            dynamic_texture_pool: None,
        }
    }
}
