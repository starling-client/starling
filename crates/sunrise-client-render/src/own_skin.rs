//! The player's own skin (Options > Skin Customization): the PNGs of `skins/` beside options.txt,
//! the one chosen and its model, kept in sunrise-player.txt (`offline_name`), and the image read
//! as vanilla reads a downloaded skin (`SkinTextureDownloader.processLegacySkin`: 64x64, or a
//! legacy 64x32 laid out anew). Only this game draws it: other players see the skin the server
//! sends for the profile, as vanilla's protocol has it, unless a Microsoft account uploads it to
//! its profile (`upload_png`).
//!
//! Each of Starling's profiles (Microsoft, Ely.by, offline) keeps its own choice, as Sunrise
//! Android's wardrobe keeps one per account (`SkinChoice::for_profile`).
use crate::offline_name;
use std::io::Read;
use std::path::Path;

/// The folder beside options.txt holding the skins to choose from.
pub const DIR: &str = "skins";
const SKIN: &str = "skin";
const MODEL: &str = "model";
/// Each profile's choice, beside options.txt: `{"<profile key>": {"skin": "a.png", "model":
/// "slim"}}`, `skin` null for the profile's own (its account's, else the default one).
pub const PROFILES: &str = "sunrise-skins.json";
/// The largest skin file read for an upload, as Horizon Android's import takes.
const MAX_UPLOAD: u64 = 1024 * 1024;

/// The chosen skin: a file of `skins/` (None: the default skin) and its model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkinChoice {
    pub file: Option<String>,
    /// The slim arms (Alex's) rather than the classic ones (Steve's).
    pub slim: bool,
}

impl SkinChoice {
    /// The choice kept in `dir`'s sunrise-player.txt.
    pub fn load(dir: &Path) -> Self {
        Self {
            file: offline_name::value(dir, SKIN).filter(|f| is_file_name(f)),
            slim: offline_name::value(dir, MODEL).as_deref() == Some("slim"),
        }
    }

    /// Keep the choice in `dir`'s sunrise-player.txt, beside the name.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let model = if self.slim { "slim" } else { "classic" };
        offline_name::set_values(dir, &[(SKIN, self.file.as_deref()), (MODEL, Some(model))])
    }

    /// The choice of the profile `key` (the accounts' profile key), kept in [`PROFILES`]; for a
    /// profile that made none yet, or with no profiles, the one kept beside the name
    /// ([`Self::load`]), which every profile wore before they had their own.
    pub fn for_profile(dir: &Path, key: Option<&str>) -> Self {
        Self::saved_for_profile(dir, key).unwrap_or_else(|| Self::load(dir))
    }

    pub fn saved_for_profile(dir: &Path, key: Option<&str>) -> Option<Self> {
        profile_choice(&read_profiles(dir), key?)
    }

    pub fn for_game(dir: &Path, key: Option<&str>) -> Self {
        read_profiles(dir)
            .get(key.unwrap_or(""))
            .and_then(|entry| entry.get("wardrobe"))
            .and_then(read_choice)
            .unwrap_or_else(|| Self::for_profile(dir, key))
    }

    pub fn save_for_game(&self, dir: &Path, key: Option<&str>) -> std::io::Result<()> {
        let mut all = read_profiles(dir);
        let model = if self.slim { "slim" } else { "classic" };
        profile_entry(&mut all, key.unwrap_or("")).insert(
            "wardrobe".into(),
            serde_json::json!({ "skin": self.file, "model": model }),
        );
        write_profiles(dir, &all)
    }

    /// Keep the choice for the profile `key`; with none, beside the name ([`Self::save`]).
    pub fn save_for_profile(&self, dir: &Path, key: Option<&str>) -> std::io::Result<()> {
        let Some(key) = key else {
            return self.save(dir);
        };
        let mut all = read_profiles(dir);
        let model = if self.slim { "slim" } else { "classic" };
        let entry = profile_entry(&mut all, key);
        entry.insert(SKIN.into(), serde_json::json!(self.file));
        entry.insert(MODEL.into(), serde_json::json!(model));
        write_profiles(dir, &all)
    }
}

/// The local override of the account's cape.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapeChoice {
    #[default]
    Account,
    Hidden,
    Local(String),
}

impl CapeChoice {
    pub fn load(dir: &Path, key: Option<&str>) -> Option<Self> {
        let all = read_profiles(dir);
        serde_json::from_value(all.get(key.unwrap_or(""))?.get("cape")?.clone()).ok()
    }

    pub fn save(&self, dir: &Path, key: Option<&str>) -> std::io::Result<()> {
        let mut all = read_profiles(dir);
        profile_entry(&mut all, key.unwrap_or("")).insert(
            "cape".into(),
            serde_json::to_value(self).map_err(std::io::Error::other)?,
        );
        write_profiles(dir, &all)
    }
}

fn profile_entry<'a>(
    all: &'a mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> &'a mut serde_json::Map<String, serde_json::Value> {
    let entry = all.entry(key).or_insert_with(|| serde_json::json!({}));
    if !entry.is_object() {
        *entry = serde_json::json!({});
    }
    entry.as_object_mut().unwrap()
}

fn write_profiles(
    dir: &Path,
    all: &serde_json::Map<String, serde_json::Value>,
) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(all).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(dir)?;
    let part = dir.join(format!("{PROFILES}.part"));
    std::fs::write(&part, text + "\n")?;
    std::fs::rename(part, dir.join(PROFILES))
}

/// Where the playing profile's skin can go besides this game (Skin Customization's account rows).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Cloud {
    /// An offline profile, or none: the skin stays in this game.
    #[default]
    Local,
    /// A Microsoft account: the chosen skin can go to its profile; how the last upload went.
    Microsoft(Upload),
    /// An Ely.by account: its skin is changed on ely.by, which takes no uploads from launchers.
    Ely,
}

/// A skin's upload to a Microsoft account's profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Upload {
    #[default]
    None,
    Sending,
    Sent,
    /// Why not: a lang key, or the reason itself.
    Failed(String),
}

/// [`PROFILES`] as it is: empty when missing, too large or not an object of the profiles.
fn read_profiles(dir: &Path) -> serde_json::Map<String, serde_json::Value> {
    let mut text = String::new();
    let read = std::fs::File::open(dir.join(PROFILES))
        .and_then(|file| file.take(256 * 1024).read_to_string(&mut text));
    match read.ok().and_then(|_| serde_json::from_str(&text).ok()) {
        Some(serde_json::Value::Object(all)) => all,
        _ => serde_json::Map::new(),
    }
}

/// The profile `key`'s choice in [`PROFILES`], if it made one.
fn profile_choice(
    all: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<SkinChoice> {
    read_choice(all.get(key)?)
}

fn read_choice(entry: &serde_json::Value) -> Option<SkinChoice> {
    let entry = entry.as_object()?;
    if !entry.contains_key(SKIN) {
        return None;
    }
    Some(SkinChoice {
        file: entry
            .get(SKIN)
            .and_then(serde_json::Value::as_str)
            .filter(|f| is_file_name(f))
            .map(str::to_owned),
        slim: entry.get(MODEL).and_then(serde_json::Value::as_str) == Some("slim"),
    })
}

/// What the page calls a skin file: its name without `.png`.
pub fn shown_name(file: &str) -> &str {
    match file.len().checked_sub(4) {
        Some(at) if file.is_char_boundary(at) && file[at..].eq_ignore_ascii_case(".png") => {
            &file[..at]
        }
        _ => file,
    }
}

/// A plain file name of `skins/`, never a path out of it.
fn is_file_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != ".."
}

/// Whether `bytes` start as a PNG file does.
pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

/// A PNG's width and height, from its header (IHDR) alone.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !is_png(bytes) || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let int = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    Some((int(16)?, int(20)?))
}

/// Why an image of this size is no skin; None for 64x64 and the legacy 64x32.
fn size_error(width: u32, height: u32) -> Option<String> {
    (width != 64 || height != 64 && height != 32)
        .then(|| format!("it is {width}x{height}, and a skin is 64x64 (or 64x32)"))
}

/// The skins to choose from: the PNG files of `skins` of a skin's size, by name.
pub fn list(skins: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(skins) else {
        return Vec::new();
    };
    let mut files: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.to_ascii_lowercase().ends_with(".png") || !entry.file_type().ok()?.is_file() {
                return None;
            }
            let mut header = [0u8; 24];
            std::fs::File::open(entry.path())
                .ok()?
                .read_exact(&mut header)
                .ok()?;
            let (width, height) = png_size(&header)?;
            size_error(width, height).is_none().then_some(name)
        })
        .collect();
    files.sort_by_key(|f| f.to_lowercase());
    files
}

/// The face and hat pixels of a skin, packed in the HUD's color format.
pub(super) type Thumbnail = [[u32; 64]; 2];

pub(super) fn thumbnail(skins: &Path, file: &str) -> Option<Thumbnail> {
    if !is_file_name(file) {
        return None;
    }
    let mut png = Vec::new();
    std::fs::File::open(skins.join(file))
        .ok()?
        .take(MAX_UPLOAD + 1)
        .read_to_end(&mut png)
        .ok()?;
    if png.len() as u64 > MAX_UPLOAD {
        return None;
    }
    let skin = decode(&png).ok()?;
    Some([8, 40].map(|left| {
        std::array::from_fn(|i| {
            let at = ((8 + i / 8) * 64 + left + i % 8) * 4;
            let [r, g, b, a] = skin[at..at + 4].try_into().unwrap();
            crate::hud::rgba(r, g, b, a)
        })
    }))
}

/// A skin PNG as the game draws it, 64x64 RGBA; why not, for a file that is no skin.
pub fn decode(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if !is_png(bytes) {
        return Err("it is not a PNG image".into());
    }
    // The header first: a photo picked by mistake is not decoded whole.
    if let Some(error) = png_size(bytes).and_then(|(w, h)| size_error(w, h)) {
        return Err(error);
    }
    let (width, height, rgba) =
        crate::hud::decode_bytes(bytes).map_err(|e| format!("it could not be read ({e})"))?;
    if let Some(error) = size_error(width, height) {
        return Err(error);
    }
    crate::skins::process_legacy_skin(width, height, &rgba)
        .ok_or_else(|| format!("it is {width}x{height}, and a skin is 64x64 (or 64x32)"))
}

/// The file `file` of `skins`, read for drawing.
pub fn load(skins: &Path, file: &str) -> Result<Vec<u8>, String> {
    if !is_file_name(file) {
        return Err(format!("{file} is not a file of the skins folder"));
    }
    let bytes =
        std::fs::read(skins.join(file)).map_err(|e| format!("could not read {file}: {e}"))?;
    decode(&bytes).map_err(|e| format!("{file} is not a skin: {e}"))
}

/// The file `file` of `skins` as an account's service takes a skin: checked as [`load`] checks
/// it, then written anew at its own size (64x64, or a legacy 64x32 kept so, which the services
/// take as it is), so that no text or other chunk of the original file leaves the device.
pub fn upload_png(skins: &Path, file: &str) -> Result<Vec<u8>, String> {
    if !is_file_name(file) {
        return Err(format!("{file} is not a file of the skins folder"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(skins.join(file))
        .and_then(|f| f.take(MAX_UPLOAD + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("could not read {file}: {e}"))?;
    if bytes.len() as u64 > MAX_UPLOAD {
        return Err(format!("{file} is larger than 1 MiB"));
    }
    decode(&bytes).map_err(|e| format!("{file} is not a skin: {e}"))?;
    let (width, height, rgba) =
        crate::hud::decode_bytes(&bytes).map_err(|e| format!("{file} could not be read ({e})"))?;
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&rgba))
        .map_err(|e| format!("{file} could not be written anew ({e})"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("sunrise-own-skin-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn png_of(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn choosing_a_cape_keeps_the_profiles_skin_and_legacy_fallback() {
        let dir = scratch("cape");
        let legacy = SkinChoice {
            file: Some("legacy.png".into()),
            slim: false,
        };
        legacy.save(&dir).unwrap();
        CapeChoice::Hidden.save(&dir, Some("profile-a")).unwrap();
        assert_eq!(SkinChoice::for_profile(&dir, Some("profile-a")), legacy);
        assert_eq!(CapeChoice::load(&dir, Some("profile-b")), None);

        let chosen = SkinChoice {
            file: Some("my-skin.png".into()),
            slim: true,
        };
        chosen.save_for_profile(&dir, Some("profile-a")).unwrap();
        assert_eq!(
            CapeChoice::load(&dir, Some("profile-a")),
            Some(CapeChoice::Hidden)
        );
        let cape = CapeChoice::Local("my-cape".into());
        cape.save(&dir, Some("profile-a")).unwrap();
        assert_eq!(SkinChoice::for_profile(&dir, Some("profile-a")), chosen);
        assert_eq!(CapeChoice::load(&dir, Some("profile-a")), Some(cape));
        legacy.save_for_game(&dir, Some("profile-a")).unwrap();
        chosen.save_for_profile(&dir, Some("profile-a")).unwrap();
        assert_eq!(SkinChoice::for_game(&dir, Some("profile-a")), legacy);
        assert_eq!(SkinChoice::for_profile(&dir, Some("profile-a")), chosen);
        fs::remove_dir_all(dir).unwrap();
    }

    fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
        let at = (y * 64 + x) * 4;
        rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn a_legacy_skin_gets_the_64x64_layout() {
        // A 64x32 skin: the right leg's front (4..8, 20..32) and arm's front (44..48, 20..32),
        // a see-through head and a fully opaque hat layer.
        let mut rgba = vec![0u8; 64 * 32 * 4];
        let mut set = |x: usize, y: usize, p: [u8; 4]| {
            let at = (y * 64 + x) * 4;
            rgba[at..at + 4].copy_from_slice(&p);
        };
        for y in 0..32 {
            for x in 32..64 {
                set(x, y, [9, 9, 9, 255]);
            }
        }
        set(4, 20, [255, 0, 0, 255]);
        set(7, 20, [0, 0, 255, 255]);
        set(44, 20, [0, 255, 0, 255]);
        set(8, 8, [1, 2, 3, 0]);
        let skin = decode(&png_of(64, 32, &rgba)).unwrap();
        assert_eq!(skin.len(), 64 * 64 * 4);
        // The left leg (20..24, 52..64) and left arm (36..40, 52..64) mirror the right ones.
        assert_eq!(pixel(&skin, 23, 52), [255, 0, 0, 255]);
        assert_eq!(pixel(&skin, 20, 52), [0, 0, 255, 255]);
        assert_eq!(pixel(&skin, 39, 52), [0, 255, 0, 255]);
        // The head is opaque; an opaque hat layer is cleared (doNotchTransparencyHack).
        assert_eq!(pixel(&skin, 8, 8), [1, 2, 3, 255]);
        assert_eq!(pixel(&skin, 40, 8)[3], 0);
        // The legacy image's own pixels stay where they were.
        assert_eq!(pixel(&skin, 4, 20), [255, 0, 0, 255]);
    }

    #[test]
    fn only_skin_sized_pngs_are_skins() {
        let square = vec![200u8; 64 * 64 * 4];
        assert_eq!(decode(&png_of(64, 64, &square)).unwrap().len(), 64 * 64 * 4);
        let error = decode(&png_of(32, 32, &[0; 32 * 32 * 4])).unwrap_err();
        assert!(error.contains("32x32"), "{error}");
        assert!(decode(&png_of(128, 128, &[0; 128 * 128 * 4])).is_err());
        assert!(decode(b"PK\x03\x04 a zip").is_err());

        let d = scratch("list");
        fs::write(d.join("steve.png"), png_of(64, 64, &square)).unwrap();
        fs::write(d.join("Old.PNG"), png_of(64, 32, &[0; 64 * 32 * 4])).unwrap();
        fs::write(d.join("big.png"), png_of(128, 128, &[0; 128 * 128 * 4])).unwrap();
        fs::write(d.join("notes.txt"), b"hello").unwrap();
        fs::write(d.join("fake.png"), b"not an image").unwrap();
        let files = list(&d);
        assert_eq!(files, ["Old.PNG", "steve.png"]);
        assert_eq!(load(&d, "steve.png").unwrap().len(), 64 * 64 * 4);
        assert!(load(&d, "../steve.png").is_err());
        assert!(load(&d, "big.png").is_err());
        assert_eq!(shown_name("Old.PNG"), "Old");
        assert_eq!(shown_name("é"), "é");
        // The cycle: the default, each skin, the default again.
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn each_profile_keeps_its_own_choice_and_the_old_one_stays_the_fallback() {
        let d = scratch("profiles");
        let before = SkinChoice {
            file: Some("old.png".into()),
            slim: false,
        };
        before.save(&d).unwrap();
        // A profile that chose nothing yet wears the choice made before profiles had their own.
        assert_eq!(SkinChoice::for_profile(&d, Some("offline:Alex")), before);
        let alex = SkinChoice {
            file: Some("alex.png".into()),
            slim: true,
        };
        alex.save_for_profile(&d, Some("offline:Alex")).unwrap();
        // The account's own skin, chosen for the Microsoft profile, is a choice of its own too.
        let account = SkinChoice::default();
        account
            .save_for_profile(&d, Some("069a79f444e94726a5befca90e38aaf5"))
            .unwrap();
        assert_eq!(SkinChoice::for_profile(&d, Some("offline:Alex")), alex);
        assert_eq!(
            SkinChoice::for_profile(&d, Some("069a79f444e94726a5befca90e38aaf5")),
            account
        );
        assert_eq!(SkinChoice::for_profile(&d, Some("ely069a")), before);
        // No profiles: the choice beside the name, untouched by the profiles'.
        assert_eq!(SkinChoice::for_profile(&d, None), before);
        let none = SkinChoice {
            file: Some("plain.png".into()),
            slim: true,
        };
        none.save_for_profile(&d, None).unwrap();
        assert_eq!(SkinChoice::load(&d), none);
        assert_eq!(SkinChoice::for_profile(&d, Some("offline:Alex")), alex);
        // A path is no file of the folder; a damaged file is no choice at all.
        fs::write(
            d.join(PROFILES),
            r#"{"offline:Alex": {"skin": "../x.png", "model": "slim"}}"#,
        )
        .unwrap();
        assert_eq!(
            SkinChoice::for_profile(&d, Some("offline:Alex")),
            SkinChoice {
                file: None,
                slim: true
            }
        );
        fs::write(d.join(PROFILES), "not json").unwrap();
        assert_eq!(SkinChoice::for_profile(&d, Some("offline:Alex")), none);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn an_upload_is_the_skin_written_anew_at_its_own_size() {
        let d = scratch("upload");
        let mut rgba = vec![0u8; 64 * 32 * 4];
        rgba[..4].copy_from_slice(&[10, 20, 30, 255]);
        let mut legacy = png_of(64, 32, &rgba);
        // A text chunk the original carries before its end: the upload leaves it behind.
        let end = legacy.len() - 12;
        let text = b"tEXtAuthor\0someone";
        let mut chunk = (text.len() as u32 - 4).to_be_bytes().to_vec();
        chunk.extend_from_slice(text);
        chunk.extend_from_slice(&[0, 0, 0, 0]);
        legacy.splice(end..end, chunk);
        fs::write(d.join("old.png"), &legacy).unwrap();
        let sent = upload_png(&d, "old.png").unwrap();
        let (width, height, pixels) = crate::hud::decode_bytes(&sent).unwrap();
        assert_eq!((width, height), (64, 32));
        assert_eq!(pixels, rgba);
        assert!(!sent.windows(6).any(|w| w == b"Author"));
        assert!(upload_png(&d, "../old.png").is_err());
        fs::write(d.join("big.png"), png_of(128, 128, &[0; 128 * 128 * 4])).unwrap();
        assert!(upload_png(&d, "big.png").is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_choice_is_kept_beside_the_name() {
        let d = scratch("choice");
        assert_eq!(SkinChoice::load(&d), SkinChoice::default());
        offline_name::choose(&d, "Alex_9").unwrap();
        let choice = SkinChoice {
            file: Some("alex.png".into()),
            slim: true,
        };
        choice.save(&d).unwrap();
        assert_eq!(SkinChoice::load(&d), choice);
        assert_eq!(offline_name::chosen(&d).as_deref(), Some("Alex_9"));
        let default = SkinChoice::default();
        default.save(&d).unwrap();
        assert_eq!(SkinChoice::load(&d), default);
        assert_eq!(
            fs::read_to_string(d.join(offline_name::FILE)).unwrap(),
            "name:Alex_9\nchosen:true\nmodel:classic\n"
        );
        // A path in the file is not a skin of the folder.
        fs::write(d.join(offline_name::FILE), "skin:../x.png\nmodel:slim\n").unwrap();
        assert_eq!(
            SkinChoice::load(&d),
            SkinChoice {
                file: None,
                slim: true
            }
        );
        let _ = fs::remove_dir_all(&d);
    }
}
