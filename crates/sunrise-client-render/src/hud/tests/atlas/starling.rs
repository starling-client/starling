//! Starling's part of atlas.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
pub(super) fn a_png_with_wrong_checksums_still_reads() {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 128])
            .unwrap();
    }
    // Every chunk's CRC broken, as a protected pack ships its icons.
    let mut pos = 8;
    while pos + 12 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        png[pos + 8 + len] ^= 0xFF;
        pos += 12 + len;
    }
    let (w, h, rgba) = decode_bytes(&png).unwrap();
    assert_eq!((w, h, rgba), (2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]));
}

#[test]
pub(super) fn a_small_gpu_gets_the_largest_textures_at_half_size() {
    // A 1024 atlas keeps 512 rows of glyphs, so 512 rows of textures.
    let image = |name: &str, side: u32| {
        let pixels = vec![0; (side * side * 4) as usize];
        (name.to_owned(), side, side, pixels)
    };
    let mut images = vec![
        image("entity/big", 600),
        image("sunrise/slot", 300),
        image("gui/small", 16),
    ];
    let mut animated = Vec::new();
    let none = HashSet::new();
    let mut full = HashMap::new();
    HudAtlas::layout_within(&mut images, &mut animated, &none, &none, 1024, &mut full).unwrap();
    let sizes: HashMap<_, _> = images.iter().map(|i| (i.0.as_str(), i.1)).collect();
    assert_eq!(sizes["entity/big"], 300);
    assert_eq!(sizes["sunrise/slot"], 300);
    assert_eq!(sizes["gui/small"], 16);
    assert_eq!(full, HashMap::from([("entity/big".to_owned(), (600, 600))]));
    // What is written by the texel never shrinks, so that much of it does not fit.
    let mut slots = vec![image("sunrise/huge", 600)];
    assert!(
        HudAtlas::layout_within(&mut slots, &mut animated, &none, &none, 1024, &mut full).is_err()
    );
}

#[test]
pub(super) fn the_interface_shrinks_after_the_rest() {
    let image = |name: &str, side: u32| {
        let pixels = vec![0; (side * side * 4) as usize];
        (name.to_owned(), side, side, pixels)
    };
    let mut images = vec![image("gui/logo", 600), image("entity/big", 400)];
    let none = HashSet::new();
    let mut full = HashMap::new();
    HudAtlas::layout_within(&mut images, &mut Vec::new(), &none, &none, 1024, &mut full).unwrap();
    let sizes: HashMap<_, _> = images.iter().map(|i| (i.0.as_str(), i.1)).collect();
    assert_eq!(sizes["entity/big"], 50);
    assert_eq!(sizes["gui/logo"], 300);
}

#[test]
pub(super) fn half_size_keeps_every_other_texel() {
    let data: Vec<u8> = (0..9u8).flat_map(|i| [i, i, i, 255]).collect();
    let half = halved(&("a".to_owned(), 3, 3, data));
    assert_eq!((half.1, half.2), (2, 2));
    let kept: Vec<u8> = half.3.chunks(4).map(|p| p[0]).collect();
    assert_eq!(kept, [0, 2, 6, 8]);
}

#[test]
pub(super) fn settings_glyph_and_plate_share_one_alpha_and_a_rounded_bore() {
    let images = vec![(
        "starling:gui/sprites/touch/custom_blank".into(),
        22,
        22,
        vec![128; 22 * 22 * 4],
    )];
    let mut images = images;
    for pixel in images[0].3.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    let baked = android_utility_sprites(&images);
    let gear = &baked
        .iter()
        .find(|i| i.0 == "starling:android-controls/settings")
        .unwrap()
        .3;
    assert!(gear.as_chunks::<4>().0.iter().all(|p| p[3] == 204));
    // The central bore is five texels wide and five high, with dark teeth
    // above it. Its alpha is the plate's, rather than two blended layers.
    for y in 8..13 {
        for x in 8..13 {
            let corner = (x == 8 || x == 12) && (y == 8 || y == 12);
            let ink = if corner { 0 } else { 128 };
            assert_eq!(
                &gear[(y * 22 + x) * 4..(y * 22 + x) * 4 + 4],
                &[ink, ink, ink, 204]
            );
        }
    }
    assert_eq!(
        &gear[(4 * 22 + 10) * 4..(4 * 22 + 10) * 4 + 4],
        &[0, 0, 0, 204]
    );
}

#[test]
pub(super) fn downloaded_controls_load_even_after_prepared_archive_was_written() {
    let root = std::env::temp_dir().join(format!("sunrise-control-overlay-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("sunrise-resources.json"), b"{}").unwrap();
    write_png(&root, "gui/sprites/widget/button", 200, 20);
    sunrise_core::prepared_files::write(&root).unwrap();
    // Downloading touch assets does not rewrite the large vanilla archive.
    write_png(&root, "gui/bedrock/gui", 256, 256);
    write_png(&root, "gui/bedrock/ui/attack", 32, 32);
    let utilities = root.join("assets/starling/textures/gui/sprites/touch");
    fs::create_dir_all(&utilities).unwrap();
    fs::copy(
        root.join("assets/minecraft/textures/gui/bedrock/ui/attack.png"),
        utilities.join("pause.png"),
    )
    .unwrap();
    let atlas = HudAtlas::build(&ResourceSource::directory(root.clone())).unwrap();
    assert_eq!(atlas.sprites["gui/bedrock/gui"].w, 256);
    assert_eq!(atlas.sprites["gui/bedrock/ui/attack"].h, 32);
    assert_eq!(atlas.sprites["starling:gui/sprites/touch/pause"].w, 32);
    let _ = fs::remove_dir_all(root);
}

#[test]
pub(super) fn small_atlas_preserves_gui_before_reducing_other_sprites() {
    let mut images = vec![
        (
            "entity/large".to_string(),
            1024,
            1024,
            vec![255; 1024 * 1024 * 4],
        ),
        ("gui/panel".to_string(), 512, 256, vec![255; 512 * 256 * 4]),
    ];
    let mut full_sizes = HashMap::new();
    HudAtlas::fit_mobile_atlas(&mut images, &mut full_sizes, &HashMap::new(), 1024).unwrap();
    let gui = images.iter().find(|i| i.0 == "gui/panel").unwrap();
    assert_eq!((gui.1, gui.2), (512, 256));
    assert!(!full_sizes.contains_key("gui/panel"));
    assert!(full_sizes.contains_key("entity/large"));
}

#[test]
pub(super) fn a_small_atlas_of_interface_sprites_alone_reduces_them_rather_than_failing() {
    // What a phone whose GPU takes 4096 sees with the default resources: the interface
    // alone is over the limit, and the glyphs and the run-time slots keep their size.
    let image =
        |name: &str, w: u32, h: u32| (name.to_string(), w, h, vec![255; (w * h * 4) as usize]);
    let mut images = vec![
        image("gui/title/background", 1024, 1024),
        image("gui/panel", 512, 512),
        image("font/ascii", 256, 256),
        ("sunrise/map0".to_string(), 128, 128, Vec::new()),
    ];
    let mut full_sizes = HashMap::new();
    HudAtlas::fit_mobile_atlas(&mut images, &mut full_sizes, &HashMap::new(), 1024).unwrap();
    let of = |name: &str| {
        let i = images.iter().find(|i| i.0 == name).unwrap();
        (i.1, i.2)
    };
    assert!(of("gui/title/background").0 < 1024);
    // Drawn at its old size, with fewer texels.
    assert_eq!(full_sizes["gui/title/background"], (1024, 1024));
    assert_eq!(of("font/ascii"), (256, 256));
    assert_eq!(of("sunrise/map0"), (128, 128));
    assert!(!full_sizes.contains_key("font/ascii"));
    assert!(!full_sizes.contains_key("sunrise/map0"));
}

#[test]
pub(super) fn mobile_layout_places_varied_rectangles_without_overlap() {
    let mut seed = 0x5a17u32;
    let mut images = Vec::new();
    for i in 0..160 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let w = 4 + seed % 53;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let h = 4 + seed % 53;
        images.push((format!("sprite{i}"), w, h, vec![]));
    }
    images.sort_by_key(|image| std::cmp::Reverse((image.2, image.1)));
    let (width, positions, bottom) = HudAtlas::mobile_layout(&images, 1024).unwrap();
    for (i, &(x, y)) in positions.iter().enumerate() {
        let (_, w, h, _) = &images[i];
        assert!(x + w < width && y + h < bottom);
        for (j, &(other_x, other_y)) in positions.iter().enumerate().take(i) {
            assert!(
                x + w < other_x
                    || other_x + images[j].1 < x
                    || y + h < other_y
                    || other_y + images[j].2 < y
            );
        }
    }
    assert!(bottom + GLYPH_ROWS <= 1024);
}

#[test]
pub(super) fn mobile_layout_uses_the_gap_below_a_short_neighbour() {
    let images = vec![
        ("tall".into(), 512, 500, vec![]),
        ("short".into(), 500, 250, vec![]),
        ("below".into(), 250, 240, vec![]),
    ];
    let (_, positions, bottom) = HudAtlas::mobile_layout(&images, 1024).unwrap();
    assert_eq!(positions, [(0, 0), (513, 0), (513, 251)]);
    assert_eq!(bottom, 501);
    assert!(bottom + GLYPH_ROWS <= 1024);
}
