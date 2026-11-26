use super::*;
use std::fs;

fn manifest() -> Manifest {
    Manifest::parse(
        r#"
id = "runes"
name = "Runes"
version = "1.0.0"
authors = ["Mod Author"]
homepage = "https://example.org/runes"
issues = "https://example.org/runes/issues"
"#,
    )
    .unwrap()
}

const TEXT: &str = "summary = 'Summary'\ndescription = 'Description'\n";
const PNG: &[u8] = include_bytes!("../../tests/fixtures/page.png");
const JPEG: &[u8] = include_bytes!("../../tests/fixtures/page.jpg");

#[test]
fn page_fields_default_to_manifest_and_explicit_values_win() {
    let manifest = manifest();
    let page = Page::parse(TEXT, &manifest).unwrap();
    assert_eq!(page.name, "Runes");
    assert_eq!(page.authors, ["Mod Author"]);
    assert_eq!(page.homepage.as_deref(), Some("https://example.org/runes"));
    assert_eq!(
        page.issues.as_deref(),
        Some("https://example.org/runes/issues")
    );
    let page = Page::parse(
        &format!("{TEXT}name = 'Rune Stones'\nauthors = []\nhomepage = 'https://example.com'\nissues = 'https://example.com/issues'\n"),
        &manifest,
    ).unwrap();
    assert_eq!(page.name, "Rune Stones");
    assert!(page.authors.is_empty());
    assert_eq!(page.homepage.as_deref(), Some("https://example.com"));
    assert_eq!(page.issues.as_deref(), Some("https://example.com/issues"));
}

#[test]
fn page_refuses_blank_names_and_invalid_links_after_resolving_defaults() {
    let manifest = manifest();
    for (fields, rejected) in [
        ("name = ''", "name"),
        ("authors = ['Good author', ' ']", "author"),
        ("homepage = 'https://'", "homepage"),
        ("homepage = 'https://user@example.org'", "homepage"),
        ("issues = 'http://user:secret@example.org'", "issues"),
        ("issues = 'http://example.org/a b'", "issues"),
        (r"issues = 'https://example.org\issues'", "issues"),
    ] {
        let error = Page::parse(&format!("{TEXT}{fields}\n"), &manifest).unwrap_err();
        assert!(error.contains(rejected), "{error}");
    }

    // The package parser accepts this prefix, so the effective page must be checked again.
    let inherited =
        Manifest::parse("id = 'runes'\nname = 'Runes'\nversion = '1'\nissues = 'https://'\n")
            .unwrap();
    assert!(
        Page::parse(TEXT, &inherited)
            .unwrap_err()
            .contains("issues")
    );
    assert!(
        Page::parse(
            &format!("{TEXT}issues = 'http://example.org/issues'"),
            &inherited
        )
        .is_ok()
    );
}

#[test]
fn page_refuses_unknown_root_and_screenshot_fields() {
    for fields in [
        "summmary = 'extra'",
        "[[screenshots]]\npath = 'page/runes.jpg'\ntitle = 'extra'",
    ] {
        let error = Page::parse(&format!("{TEXT}{fields}\n"), &manifest()).unwrap_err();
        assert!(error.contains("unknown field"), "{error}");
    }
}

#[test]
fn screenshot_paths_are_relative_and_strictly_below_page() {
    let manifest = manifest();
    let long = format!("page/{}.jpg", "x".repeat(100));
    for (path, expected) in [
        ("runes.jpg", "under page/"),
        ("/page/runes.jpg", "under page/"),
        ("page/../runes.jpg", "under page/"),
        ("page/./runes.jpg", "under page/"),
        ("page/", "under page/"),
        ("page/C:/runes.jpg", "under page/"),
        ("page/sub\\runes.jpg", "under page/"),
        ("page/.. /runes.jpg", "under page/"),
        (&long, "not a ustar path"),
    ] {
        let error = Page::parse(
            &format!("{TEXT}[[screenshots]]\npath = '{path}'\n"),
            &manifest,
        )
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn screenshots_must_decode_and_match_their_file_extension() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("page/nested")).unwrap();
    for (name, bytes, signature_len) in [("image.png", PNG, 8), ("image.jpg", JPEG, 3)] {
        let page = Page::parse(
            &format!(
                "{TEXT}[[screenshots]]\npath = 'page/nested/{name}'\ncaption = 'Rune Stones'\n"
            ),
            &manifest(),
        )
        .unwrap();
        let shot = &page.screenshots[0];
        assert_eq!(shot.caption.as_deref(), Some("Rune Stones"));
        let path = root.path().join(shot.path());
        fs::write(&path, bytes).unwrap();
        assert_eq!(shot.read(root.path()).unwrap(), bytes);
        for truncated in [&bytes[..signature_len], &bytes[..bytes.len() - 1]] {
            fs::write(&path, truncated).unwrap();
            assert!(shot.read(root.path()).unwrap_err().contains("unreadable"));
            assert!(shot.check_image(truncated).is_err());
        }
    }
    assert!(check_screenshot(Path::new("image.jpeg"), JPEG).is_ok());
    assert!(
        check_screenshot(Path::new("image.jpg"), PNG)
            .unwrap_err()
            .contains("extension")
    );
    assert!(check_screenshot(Path::new("image.png"), b"not an image").is_err());
}

#[test]
fn a_jpeg_too_large_to_decode_is_refused_from_its_header() {
    // The fixture with its frame header claiming 16000 x 16000 pixels: over the decoded limit,
    // refused before the decoder allocates the picture.
    let mut huge = JPEG.to_vec();
    let frame = huge
        .windows(2)
        .position(|marker| marker[0] == 0xff && matches!(marker[1], 0xc0 | 0xc2))
        .expect("the fixture has a frame header");
    huge[frame + 5..frame + 9].copy_from_slice(&[0x3e, 0x80, 0x3e, 0x80]);
    let error = check_screenshot(Path::new("image.jpg"), &huge).unwrap_err();
    assert!(
        error.contains(&MAX_DECODED_SCREENSHOT_BYTES.to_string()),
        "{error}"
    );
}

/// 48 x 48 pictures of ours (seeded noise over gradients), written by Pillow: one sequential
/// scan; ten progressive scans; one sequential scan in three restart intervals.
const SEQUENTIAL: &[u8] = include_bytes!("../../tests/fixtures/page-sequential.jpg");
const PROGRESSIVE: &[u8] = include_bytes!("../../tests/fixtures/page-progressive.jpg");
const RESTART: &[u8] = include_bytes!("../../tests/fixtures/page-restart.jpg");

/// `jpeg` cut inside its last scan's entropy-coded data, then closed with an end-of-image
/// marker, so the end marker is no sign that the scan was whole.
fn cut_inside_last_scan(jpeg: &[u8]) -> Vec<u8> {
    let data = jpeg
        .windows(2)
        .rposition(|m| m == [0xff, 0xda])
        .map(|sos| sos + 2 + usize::from(u16::from_be_bytes([jpeg[sos + 2], jpeg[sos + 3]])))
        .expect("a scan");
    // After the last restart marker, if the scan has any, so the cut is in its last interval.
    let data = jpeg[data..]
        .windows(2)
        .rposition(|m| m[0] == 0xff && (0xd0..=0xd7).contains(&m[1]))
        .map_or(data, |rst| data + rst + 2);
    let end = jpeg.len() - 2;
    let mut cut = data + (end - data) / 2;
    if jpeg[cut - 1] == 0xff {
        cut -= 1;
    }
    [&jpeg[..cut], &[0xff, 0xd9][..]].concat()
}

#[test]
fn whole_jpegs_of_every_layout_pass() {
    for jpeg in [JPEG, SEQUENTIAL, PROGRESSIVE, RESTART] {
        check_screenshot(Path::new("image.jpg"), jpeg).unwrap();
    }
}

#[test]
fn a_jpeg_cut_inside_a_scan_is_refused_despite_its_end_marker() {
    for (layout, jpeg) in [
        ("sequential", SEQUENTIAL),
        ("progressive", PROGRESSIVE),
        ("restart", RESTART),
    ] {
        let cut = cut_inside_last_scan(jpeg);
        assert!(cut.ends_with(&[0xff, 0xd9]) && cut.len() < jpeg.len());
        assert!(
            check_screenshot(Path::new("image.jpg"), jpeg).is_ok(),
            "{layout}"
        );
        assert!(
            check_screenshot(Path::new("image.jpg"), &cut).is_err(),
            "{layout}"
        );
    }
}

#[test]
fn a_scan_naming_a_huffman_table_past_3_is_refused_without_a_panic() {
    // The progressive fixture's first scan is DC (no AC table used) and its second is AC (no
    // DC table used): a destination past 3 in the unused nibble is still refused.
    let scans: Vec<usize> = PROGRESSIVE
        .windows(2)
        .enumerate()
        .filter(|(_, m)| *m == [0xff, 0xda])
        .map(|(at, _)| at)
        .collect();
    for (scan, nibble) in [(scans[0], 0x0f), (scans[1], 0xf0)] {
        let mut bad = PROGRESSIVE.to_vec();
        // After FF DA, the length (2 bytes) and Ns: the first component's Td/Ta byte.
        bad[scan + 6] |= nibble;
        assert!(super::jpeg::check_complete(&bad).is_err());
        assert!(check_screenshot(Path::new("image.jpg"), &bad).is_err());
    }
}

#[test]
fn a_progressive_jpeg_missing_its_later_scans_is_refused() {
    // Cut before the fourth scan: the first three are whole, but the coefficients the rest
    // carry never reach full precision.
    let fourth = PROGRESSIVE
        .windows(2)
        .enumerate()
        .filter(|(_, m)| *m == [0xff, 0xda])
        .nth(3)
        .map(|(at, _)| at)
        .expect("ten scans");
    let cut = [&PROGRESSIVE[..fourth], &[0xff, 0xd9][..]].concat();
    assert!(check_screenshot(Path::new("image.jpg"), PROGRESSIVE).is_ok());
    assert!(check_screenshot(Path::new("image.jpg"), &cut).is_err());
}

#[cfg(unix)]
#[test]
fn linked_screenshot_subfolders_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("runes.png"), PNG).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("page")).unwrap();
    let page = Page::parse(
        &format!("{TEXT}[[screenshots]]\npath = 'page/runes.png'\n"),
        &manifest(),
    )
    .unwrap();
    assert!(
        page.screenshots[0]
            .read(root.path())
            .unwrap_err()
            .contains("not a regular directory")
    );
}

#[test]
fn release_text_counts_trimmed_unicode_scalars() {
    let manifest = manifest();
    let metadata = |summary: &str, description: &str| {
        Page::parse(&format!("summary = ' {summary} '\ndescription = ' {description} '\n[[screenshots]]\npath = 'page/runes.png'\n"), &manifest).unwrap()
    };
    assert_eq!(
        metadata(&"🪨".repeat(24), &"🪨".repeat(124)).release_problems(None),
        [
            "page.toml: summary needs at least 25 characters",
            "page.toml: description needs at least 125 characters",
        ]
    );
    assert!(
        metadata(&"🪨".repeat(25), &"🪨".repeat(125))
            .release_problems(None)
            .is_empty()
    );
}

#[test]
fn release_checks_the_category_catalog_and_cap_but_allows_no_categories() {
    let catalog = Categories::parse(
        "[[category]]\nid = 'utility'\nicon = 'hammer'\nkinds = ['mod']\n\
         [[category]]\nid = 'automation'\nicon = 'cog'\nkinds = ['native-mod']\n",
    )
    .unwrap();
    let release = format!(
        "summary = '{}'\ndescription = '{}'\n",
        "s".repeat(25),
        "d".repeat(125)
    );
    let screenshots = "[[screenshots]]\npath = 'page/runestones.jpg'\n";
    let manifest = manifest();
    let check = |settings| {
        Page::parse(&format!("{release}{settings}\n{screenshots}"), &manifest)
            .unwrap()
            .release_problems(Some(&catalog))
    };
    assert!(check("").is_empty());
    assert!(check("categories = ['utility']").is_empty());
    assert_eq!(
        check("categories = ['not_a_category']"),
        ["page.toml: unknown registry category \"not_a_category\""]
    );
    assert_eq!(
        check("categories = ['utility', 'utility']"),
        ["page.toml: duplicate registry category \"utility\""]
    );
    assert_eq!(
        check("categories = ['automation']"),
        ["page.toml: registry category \"automation\" does not apply to this project kind"]
    );
    let native = Manifest::parse("id = 'runes'\nname = 'Runes'\nversion = '1'\n[native]\ngame = '26.3'\nsunrise = '0123456789abcdef0123456789abcdef01234567'\n").unwrap();
    assert!(
        Page::parse(
            &format!("{release}categories = ['automation']\n{screenshots}"),
            &native
        )
        .unwrap()
        .release_problems(Some(&catalog))
        .is_empty()
    );
    let over_cap = category_problems(
        ["a", "b", "c", "d", "e", "f", "g", "h", "i"].into_iter(),
        Kind::Mod,
        None,
    );
    assert_eq!(over_cap, ["choose at most 8 categories"]);
    assert_eq!(
        Page::parse(&release, &manifest)
            .unwrap()
            .release_problems(Some(&catalog)),
        ["page.toml: add at least one [[screenshots]] entry",]
    );
}

#[test]
fn each_screenshot_needs_its_own_portable_destination_beside_the_icon() {
    let problems = |paths: &[&str]| {
        let rows: String = paths
            .iter()
            .map(|path| format!("[[screenshots]]\npath = '{path}'\n"))
            .collect();
        let text = format!(
            "summary = '{}'\ndescription = '{}'\n{rows}",
            "a".repeat(25),
            "a".repeat(125)
        );
        Page::parse(&text, &manifest())
            .unwrap()
            .release_problems(None)
            .len()
    };
    assert_eq!(problems(&["page/a.png", "page/shots/b.jpg"]), 0);
    assert_eq!(problems(&["page/a.png", "page/a.png"]), 1);
    assert_eq!(problems(&["page/icon.png"]), 1);
    assert_eq!(problems(&["page/aux.png"]), 1);
    assert_eq!(problems(&["page/com1/a.png"]), 1);
}

#[test]
fn the_description_defaults_to_the_manifest() {
    let mut manifest = manifest();
    manifest.description = "From the manifest".into();
    let page = Page::parse("summary = 'Summary'\n", &manifest).unwrap();
    assert_eq!(page.description, "From the manifest");
    assert_eq!(
        Page::parse(TEXT, &manifest).unwrap().description,
        "Description"
    );
}

#[test]
fn a_jpeg_counts_its_coefficients_against_the_decoding_budget() {
    // The fixture's frame at 8000 x 8000 with three components, sequential and progressive: its
    // 192 MB of pixels fit the budget, but the decoder's coefficients double that, so it is
    // refused from the header, before anything is allocated.
    const { assert!(8000 * 8000 * 3 < MAX_DECODED_SCREENSHOT_BYTES) };
    for marker in [0xc0, 0xc2] {
        let mut image = JPEG.to_vec();
        let at = image
            .windows(2)
            .position(|m| m[0] == 0xff && matches!(m[1], 0xc0 | 0xc2))
            .expect("the fixture has a frame header");
        image[at + 1] = marker;
        image[at + 5..at + 9].copy_from_slice(&[0x1f, 0x40, 0x1f, 0x40]);
        let error = check_screenshot(Path::new("image.jpg"), &image).unwrap_err();
        assert!(
            error.contains(&MAX_DECODED_SCREENSHOT_BYTES.to_string()),
            "{error}"
        );
    }
}
