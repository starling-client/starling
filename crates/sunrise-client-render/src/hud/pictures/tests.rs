use super::*;
fn temp_root(label: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("sunrise-{label}-{}-{nonce}", std::process::id()))
}
fn png(root: &Path, name: &str, rgba: [u8; 4]) {
    let path = root.join(format!("assets/minecraft/textures/{name}.png"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut image = png::Encoder::new(fs::File::create(path).unwrap(), 1, 1);
    image.set_color(png::ColorType::Rgba);
    image.set_depth(png::BitDepth::Eight);
    image
        .write_header()
        .unwrap()
        .write_image_data(&rgba)
        .unwrap();
}
#[test]
fn pictures_are_lazy_shared_and_released_then_reloaded_from_the_pack() {
    let root = temp_root("pictures");
    let pack = root.join("pack");
    let defaults = root.join("defaults");
    let name = "gui/presets/isles";
    let mut cache = Cache::new(
        &crate::resource_source::ResourceSource::directory(pack.clone()),
        &crate::resource_source::ResourceSource::directory(defaults.clone()),
    );
    // Nothing has been read: creating a picture after the cache works.
    png(&defaults, name, [1, 2, 3, 255]);
    let first = cache.get(name).unwrap();
    assert_eq!(first.pixels, [1, 2, 3, 255]);
    assert!(Arc::ptr_eq(&first, &cache.get(name).unwrap()));
    let weak = Arc::downgrade(&first);
    drop(first);
    assert!(
        weak.upgrade().is_none(),
        "the atlas cache must not retain decoded pixels"
    );
    png(&pack, name, [4, 5, 6, 255]);
    fs::write(
        pack.join(format!("assets/minecraft/textures/{name}.png.mcmeta")),
        r#"{"texture":{"blur":true,"clamp":true}}"#,
    )
    .unwrap();
    let replacement = cache.get(name).unwrap();
    assert_eq!(replacement.pixels, [4, 5, 6, 255]);
    assert_eq!(
        replacement.flags,
        TextureFlags {
            blur: true,
            clamp: true
        }
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
#[ignore = "daily"]
fn realms_pictures_do_not_enlarge_the_startup_atlas() {
    let root = temp_root("realms-atlas");
    let control = HudAtlas::build(&crate::resource_source::ResourceSource::directory(
        root.clone(),
    ))
    .unwrap();
    // The fork reserves static atlas slots for panorama 0/2/3 and the Isles icon.
    // Other pictures must remain lazy, including arbitrary preset icons.
    for name in [
        "gui/title/background/panorama_1",
        "gui/title/background/panorama_4",
        "gui/title/background/panorama_5",
        "gui/presets/custom",
    ] {
        let path = root.join(format!("assets/minecraft/textures/{name}.png"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), 1024, 1024);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![255; 1024 * 1024 * 4])
            .unwrap();
    }
    let atlas = HudAtlas::build(&crate::resource_source::ResourceSource::directory(
        root.clone(),
    ))
    .unwrap();
    assert_eq!(
        (atlas.width, atlas.height, atlas.pixels.len()),
        (control.width, control.height, control.pixels.len())
    );
    assert!(!atlas.sprites.contains_key("gui/presets/custom"));
    let image = atlas
        .pictures
        .lock()
        .unwrap()
        .get("gui/presets/custom")
        .unwrap();
    assert_eq!((image.width, image.height), (1024, 1024));
    eprintln!(
        "Realms startup atlas: {}x{}, {} bytes, unchanged with four 1024x1024 pictures",
        atlas.width,
        atlas.height,
        atlas.pixels.len()
    );
    fs::remove_dir_all(&root).unwrap();
}

/// A `width` x 1 PNG of `pixels` at the resource `name` under `root`.
fn png_at(root: &Path, name: &str, pixels: &[[u8; 4]]) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut image = png::Encoder::new(fs::File::create(path).unwrap(), pixels.len() as u32, 1);
    image.set_color(png::ColorType::Rgba);
    image.set_depth(png::BitDepth::Eight);
    image
        .write_header()
        .unwrap()
        .write_image_data(pixels.as_flattened())
        .unwrap();
}

fn write(root: &Path, name: &str, bytes: &[u8]) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn folder_pack(root: &Path, name: &str) -> crate::packs::Pack {
    write(
        root,
        &format!("{name}/pack.mcmeta"),
        br#"{"pack":{"description":"test","pack_format":97}}"#,
    );
    crate::packs::pack_for(
        name.into(),
        crate::text::Text::empty(),
        crate::packs::Source::Dir(root.join(name)),
        crate::packs::RESOURCE_FORMAT,
    )
    .unwrap()
}

/// The resource stack `base` with `packs` over it, lowest first, opened afresh as a reload does.
fn stack(root: &Path, packs: &[&str]) -> HudAtlas {
    let base = crate::resource_source::ResourceSource::directory(root.join("base"));
    let packs: Vec<_> = packs.iter().map(|name| folder_pack(root, name)).collect();
    HudAtlas::build(&crate::packs::resources(base, &packs).unwrap()).unwrap()
}

/// `HudBatch::texture` of `id`, untinted.
fn texture(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    id: &str,
    at: [f32; 4],
    uv: [f32; 2],
    [src, tex]: [[i32; 2]; 2],
) {
    batch.texture(
        atlas,
        &Identifier::parse(id).unwrap(),
        at,
        uv,
        [src, tex],
        0xFFFF_FFFF,
    );
}

use crate::hud_layout::{GuiDraw, paint};

const CHAPTER: &str = "assets/demo/textures/chapter.png";

#[test]
fn a_texture_is_the_exact_resource_sampled_as_its_winning_mcmeta_says_until_a_reload() {
    let root = temp_root("gui-texture");
    // A non-GUI image of a mod's namespace, and a higher pack with only its `.mcmeta`.
    png_at(
        &root.join("base"),
        CHAPTER,
        &[[1, 2, 3, 255], [4, 5, 6, 255]],
    );
    write(
        &root.join("meta"),
        &format!("{CHAPTER}.mcmeta"),
        br#"{"texture":{"blur":true,"clamp":true}}"#,
    );
    let atlas = stack(&root, &["meta"]);
    // `innerBlit`: u from -2 / 2 to (-2 + 4) / 2, v from 0 to 2: past 1 both ways, unclamped.
    let chapter = |batch: &mut HudBatch, atlas: &HudAtlas| {
        let at = [10.0, 20.0, 32.0, 16.0];
        texture(
            batch,
            atlas,
            "demo:textures/chapter.png",
            at,
            [-2.0, 0.0],
            [[4, 2], [2, 1]],
        );
    };
    let mut batch = HudBatch::scratch([854, 480]);
    chapter(&mut batch, &atlas);
    let [picture] = batch.pictures.as_slice() else {
        panic!("one picture, not {}", batch.pictures.len())
    };
    assert_eq!(picture.pixels, [1, 2, 3, 255, 4, 5, 6, 255]);
    assert_eq!(
        picture.flags,
        TextureFlags {
            blur: true,
            clamp: true
        }
    );
    let linear = sampler(picture.flags);
    assert_eq!(linear.min_filter, wgpu::FilterMode::Linear);
    assert_eq!(linear.mag_filter, wgpu::FilterMode::Linear);
    assert_eq!(linear.address_mode_u, wgpu::AddressMode::ClampToEdge);
    assert_eq!(linear.address_mode_v, wgpu::AddressMode::ClampToEdge);
    assert_eq!(linear.mipmap_filter, wgpu::MipmapFilterMode::Nearest);
    assert_eq!(batch.blends, [(0..6, Blend::Picture(picture.id))]);
    let corner = |pos: [f32; 2]| batch.vertices.iter().find(|v| v.pos == pos).unwrap().uv;
    assert_eq!(corner([10.0, 20.0]), [-1.0, 0.0]);
    assert_eq!(corner([42.0, 20.0]), [1.0, 0.0]);
    assert_eq!(corner([42.0, 36.0]), [1.0, 2.0]);
    assert_eq!(corner([10.0, 36.0]), [-1.0, 2.0]);

    // Held for the generation: drawn again with its file gone, it is the same decoded image.
    let first = picture.clone();
    fs::remove_file(root.join("base").join(CHAPTER)).unwrap();
    let mut again = HudBatch::scratch([854, 480]);
    chapter(&mut again, &atlas);
    assert!(Arc::ptr_eq(&first, &again.pictures[0]));

    // A reload: a higher pack's image wins, and the `.mcmeta` below it no longer counts, so the
    // sampler is nearest and repeat again.
    png_at(&root.join("art"), CHAPTER, &[[9, 8, 7, 255]]);
    let reloaded = stack(&root, &["meta", "art"]);
    let mut batch = HudBatch::scratch([854, 480]);
    chapter(&mut batch, &reloaded);
    let picture = &batch.pictures[0];
    assert_ne!(picture.id, first.id);
    assert_eq!(picture.pixels, [9, 8, 7, 255]);
    assert_eq!(picture.flags, TextureFlags::default());
    let nearest = sampler(picture.flags);
    assert_eq!(nearest.min_filter, wgpu::FilterMode::Nearest);
    assert_eq!(nearest.mag_filter, wgpu::FilterMode::Nearest);
    assert_eq!(nearest.address_mode_u, wgpu::AddressMode::Repeat);
    assert_eq!(nearest.address_mode_v, wgpu::AddressMode::Repeat);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_missing_or_undecodable_texture_draws_the_missing_texture_in_its_place() {
    let root = temp_root("gui-texture-missing");
    write(
        &root.join("base"),
        "assets/demo/textures/broken.png",
        b"not a png",
    );
    let atlas = stack(&root, &[]);
    let fill = GuiDraw::Fill {
        x0: 0,
        y0: 0,
        x1: 4,
        y1: 4,
        color: 0xFFFF_FFFF,
    };
    let mut batch = HudBatch::scratch([854, 480]);
    paint(&mut batch, &atlas, std::slice::from_ref(&fill));
    // Named as an atlas sprite would be, `demo:broken` is still only the exact resource.
    for (id, x) in [
        ("demo:textures/nothing.png", 8.0),
        ("demo:textures/broken.png", 24.0),
        ("demo:broken", 40.0),
    ] {
        let at = [x, 0.0, 16.0, 16.0];
        texture(&mut batch, &atlas, id, at, [0.0, 0.0], [[16, 16], [16, 16]]);
    }
    paint(&mut batch, &atlas, std::slice::from_ref(&fill));
    // Every op drew, in order: the fills around one run of the shared missing texture.
    assert_eq!(batch.vertices.len(), 5 * 6);
    let [missing] = batch.pictures.as_slice() else {
        panic!("one missing texture, not {}", batch.pictures.len())
    };
    assert_eq!(batch.blends, [(6..24, Blend::Picture(missing.id))]);
    assert_eq!((missing.width, missing.height), (16, 16));
    // `generateMissingImage`: black top left, magenta top right.
    assert_eq!(missing.pixels[..4], [0, 0, 0, 255]);
    assert_eq!(missing.pixels[15 * 4..16 * 4], [248, 0, 248, 255]);
    assert_eq!(missing.flags, TextureFlags::default());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_texture_with_metadata_vanilla_cannot_read_is_the_missing_texture() {
    let root = temp_root("gui-texture-meta");
    let pack = root.join("pack");
    png_at(&pack, CHAPTER, &[[1, 2, 3, 255]]);
    let meta = pack.join(format!("{CHAPTER}.mcmeta"));
    let id = Identifier::parse("demo:textures/chapter.png").unwrap();
    let load = |json: &str| {
        fs::write(&meta, json).unwrap();
        // A fresh generation each time, as a reload would open.
        let mut cache = Cache::new(
            &crate::resource_source::ResourceSource::directory(pack.clone()),
            &crate::resource_source::ResourceSource::directory(root.join("defaults")),
        );
        let texture = cache.texture(&id);
        (texture.pixels.len() == 4, texture.flags)
    };
    // `ResourceMetadata.fromJsonStream` or the section's codec throws: `loadContentsSafe` loads
    // the missing texture.
    for refused in [
        "not json",
        "[]",
        r#"{"texture": true}"#,
        r#"{"texture": {"blur": "yes"}}"#,
        r#"{"texture": {"clamp": 1}}"#,
        r#"{"texture": {"mipmap_strategy": "fancy"}}"#,
        r#"{"texture": {"alpha_cutoff_bias": "high"}}"#,
    ] {
        assert_eq!(load(refused), (false, TextureFlags::default()), "{refused}");
    }
    // Absent sections, `null` fields and keys the codec does not know draw the image.
    let blur = TextureFlags {
        blur: true,
        clamp: false,
    };
    for (read, flags) in [
        ("{}", TextureFlags::default()),
        (
            r#"{"animation": {"frametime": "x"}}"#,
            TextureFlags::default(),
        ),
        (
            r#"{"texture": {"blur": true, "clamp": null, "extra": 1}}"#,
            blur,
        ),
        (
            r#"{"texture": {"blur": true, "mipmap_strategy": "dark_cutout"}}"#,
            blur,
        ),
    ] {
        assert_eq!(load(read), (true, flags), "{read}");
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_default_texture_takes_its_metadata_from_the_pack_above_it() {
    // `createStackMetadataFinder`: the topmost `.mcmeta` from the image's root or above, so a
    // pack can set the sampler of a texture only the built-in defaults hold.
    let root = temp_root("gui-texture-defaults");
    let (pack, defaults) = (root.join("pack"), root.join("defaults"));
    png_at(&defaults, CHAPTER, &[[1, 2, 3, 255]]);
    write(
        &defaults,
        &format!("{CHAPTER}.mcmeta"),
        br#"{"texture":{"blur":true}}"#,
    );
    write(
        &pack,
        &format!("{CHAPTER}.mcmeta"),
        br#"{"texture":{"clamp":true}}"#,
    );
    let mut cache = Cache::new(
        &crate::resource_source::ResourceSource::directory(pack),
        &crate::resource_source::ResourceSource::directory(defaults),
    );
    let texture = cache.texture(&Identifier::parse("demo:textures/chapter.png").unwrap());
    assert_eq!(texture.pixels, [1, 2, 3, 255]);
    assert_eq!(
        texture.flags,
        TextureFlags {
            blur: false,
            clamp: true
        }
    );
    fs::remove_dir_all(&root).unwrap();
}
