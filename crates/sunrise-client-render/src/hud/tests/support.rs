use super::super::*;

/// A scratch tree with a 2 x 1 sheet (an opaque red texel, then a transparent green one) and
/// the two atlas definitions.
pub(super) fn model_atlas_root(tag: &str, items: &str, blocks: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sunrise-hud-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let textures = root.join("assets/demo/textures/item");
    let atlases = root.join("assets/minecraft/atlases");
    std::fs::create_dir_all(&textures).unwrap();
    std::fs::create_dir_all(&atlases).unwrap();
    let mut encoder = png::Encoder::new(
        std::fs::File::create(textures.join("sheet.png")).unwrap(),
        2,
        1,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 0])
        .unwrap();
    std::fs::write(atlases.join("items.json"), items).unwrap();
    std::fs::write(atlases.join("blocks.json"), blocks).unwrap();
    root
}

pub(super) fn cut(sprite: &str, x: u8) -> String {
    format!(
        r#"{{"sources":[{{"type":"minecraft:unstitch","resource":"demo:item/sheet","divisor_x":2,
        "regions":[{{"sprite":"{sprite}","x":{x},"y":0,"width":1,"height":1}}]}}]}}"#
    )
}

/// `assets/minecraft/atlases/gui.json` as the game's resources have it.
pub(super) fn write_vanilla_gui_definition(root: &std::path::Path) {
    let atlases = root.join("assets/minecraft/atlases");
    fs::create_dir_all(&atlases).unwrap();
    fs::write(
        atlases.join("gui.json"),
        r#"{"sources":[
            {"type":"minecraft:directory","prefix":"","source":"gui/sprites"},
            {"type":"minecraft:directory","prefix":"mob_effect/","source":"mob_effect"}]}"#,
    )
    .unwrap();
}

pub(super) fn write_pixel(path: &std::path::Path, rgba: [u8; 4]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), 1, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&rgba)
        .unwrap();
}

/// Write a PNG `width x height` whose `frames` stacked rows are solid
/// colours, then its mcmeta beside it.
pub(super) fn write_strip(
    path: &Path,
    width: u32,
    height: u32,
    frames: &[[u8; 4]],
    mcmeta: Option<&str>,
) {
    let file = std::io::BufWriter::new(fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for frame in frames {
        let rows = height / frames.len() as u32;
        pixels.extend((0..width * rows).flat_map(|_| *frame));
    }
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    if let Some(mcmeta) = mcmeta {
        let meta = path.with_extension("png.mcmeta");
        fs::write(meta, mcmeta).unwrap();
    }
}

/// A resource root holding one animated block texture: a strip of two
/// 16x16 frames, red then blue, whose mcmeta starts on the blue one.
/// Each call gets its own folder: tests run side by side, and one adds
/// a strip of its own that another would otherwise find.
pub(super) fn animated_root() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("sunrise-atlas-anim-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let block = root.join("assets/minecraft/textures/block");
    fs::create_dir_all(&block).unwrap();
    write_strip(
        &block.join("test_anim.png"),
        16,
        32,
        &[[255, 0, 0, 255], [0, 0, 255, 255]],
        Some(r#"{"animation": {"frames": [1, 0]}}"#),
    );
    root
}

pub(super) fn texel_at(atlas: &HudAtlas, rect: Rect, x: u32, y: u32) -> [u8; 4] {
    let px = (rect.u0 * atlas.width as f32).round() as u32 + x;
    let py = (rect.v0 * atlas.height as f32).round() as u32 + y;
    let at = ((py * atlas.width + px) * 4) as usize;
    atlas.pixels[at..at + 4].try_into().unwrap()
}

/// A resource root with one plain item texture and these CIT files (relative path, contents).
pub(super) fn cit_root(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("sunrise-atlas-cit-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let item = root.join("assets/minecraft/textures/item");
    fs::create_dir_all(&item).unwrap();
    write_strip(&item.join("stick.png"), 16, 16, &[[90, 60, 30, 255]], None);
    let cit = root.join("assets/minecraft/optifine/cit");
    fs::create_dir_all(&cit).unwrap();
    for (rel, text) in files {
        fs::write(cit.join(rel), text).unwrap();
    }
    root
}

/// `count` images of `w x h`; layout reads only their sizes.
pub(super) fn sized(count: usize, w: u32, h: u32) -> Vec<PackedImage> {
    (0..count)
        .map(|i| (format!("i{i}"), w, h, Vec::new()))
        .collect()
}

pub(super) fn assert_inside(
    images: &[PackedImage],
    width: u32,
    placements: &[(u32, u32)],
    free: u32,
) {
    for ((_, w, h, _), (x, y)) in images.iter().zip(placements) {
        assert!(
            x + w <= width && y + h < free,
            "{x},{y} {w}x{h} outside {width}x{free}"
        );
    }
}

pub(super) fn write_png(root: &Path, name: &str, w: u32, h: u32) {
    let path = root.join(format!("assets/minecraft/textures/{name}.png"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let pixels = vec![255; (w * h * 4) as usize];
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

/// A synthetic default font: two 8x8 glyphs, 3 and 5 pixels wide.
pub(super) fn two_glyph_font(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sunrise-fonts-{name}-{}", std::process::id()));
    let font = root.join("assets/minecraft/font");
    let textures = root.join("assets/minecraft/textures/font");
    fs::create_dir_all(&font).unwrap();
    fs::create_dir_all(&textures).unwrap();
    fs::write(
        font.join("default.json"),
        r#"{"providers":[{"type":"bitmap","file":"minecraft:font/test.png","ascent":7,"chars":["ab"]}]}"#,
    )
    .unwrap();
    let mut pixels = vec![0u8; 16 * 8 * 4];
    for y in 0..8 {
        for x in (0..3).chain(8..13) {
            pixels[(y * 16 + x) * 4 + 3] = 255;
        }
    }
    let mut encoder =
        png::Encoder::new(fs::File::create(textures.join("test.png")).unwrap(), 16, 8);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    root
}

/// A pack layer that adds a font provider over the game's own resources:
/// its `font/default.json` names `minecraft:font/test.png`, which only the
/// game's root holds (`HudAtlas::build_over`'s `defaults`).
pub(super) fn pack_font_over_defaults(name: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "sunrise-font-fallback-{name}-{}",
        std::process::id()
    ));
    let pack = base.join("pack");
    let vanilla = base.join("vanilla");
    fs::create_dir_all(pack.join("assets/minecraft/font")).unwrap();
    fs::create_dir_all(vanilla.join("assets/minecraft/textures/font")).unwrap();
    fs::write(
        pack.join("assets/minecraft/font/default.json"),
        r#"{"providers":[{"type":"bitmap","file":"minecraft:font/test.png","ascent":7,"chars":["ab"]}]}"#,
    )
    .unwrap();
    let mut pixels = vec![0u8; 16 * 8 * 4];
    for y in 0..8 {
        for x in (0..3).chain(8..13) {
            pixels[(y * 16 + x) * 4 + 3] = 255;
        }
    }
    let mut encoder = png::Encoder::new(
        fs::File::create(vanilla.join("assets/minecraft/textures/font/test.png")).unwrap(),
        16,
        8,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    (pack, vanilla)
}
