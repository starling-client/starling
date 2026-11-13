//! Custom armour a resource pack draws with its core shaders, which Horizon does not run. An
//! ItemsAdder pack puts its armours side by side in the leather layer's texture, each under a
//! key colour, and its entity shader draws the one whose key is the leather's dye colour,
//! animated by the game time; any other colour is the plain leather of the first column.
use std::{collections::HashMap, path::Path};

/// The constants ItemsAdder writes into its entity shader (between `%START_CUSTOM_ARMORS_VARS%`
/// and `%END_CUSTOM_ARMORS_VARS%`): an armour is `4 v1` by `2 v1` pixels, frames advance
/// `v2` times a day per speed step, and the atlas is `width` by `height`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasShape {
    pub v1: u32,
    pub v2: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasShape {
    pub fn parse(shader: &str) -> Option<Self> {
        let define = |name: &str| {
            shader.lines().find_map(|line| {
                let mut words = line.split_whitespace();
                (words.next()? == "#define" && words.next()? == name)
                    .then(|| words.next()?.parse().ok())
                    .flatten()
            })
        };
        Some(Self {
            v1: define("V1")?,
            v2: define("V2")?,
            width: define("TEX_WIDTH")?,
            height: define("TEX_HEIGHT")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Armor {
    /// The dye colour (RGB) that picks it.
    key: u32,
    column: u32,
    frames: f32,
    speed: f32,
    /// Drawn in the dye colour as well (otherwise as it is).
    tinted: bool,
}

/// A leather layer texture holding custom armours.
#[derive(Clone, Debug, PartialEq)]
pub struct ArmorAtlas {
    columns: u32,
    rows: u32,
    v2: f32,
    armors: Vec<Armor>,
}

impl ArmorAtlas {
    /// The texture `rgba` (`width` by `height`) as such an atlas, if it is one: the shader's own
    /// test is the size, an opaque white pixel at (0, 1) and a clear one at (7, 7). Above each
    /// armour, its first row holds the key, the animation (frames, speed) and how it is
    /// coloured, each pixel's channels scaled by its alpha.
    pub fn read(shape: AtlasShape, width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        if (width, height) != (shape.width, shape.height) || shape.v1 == 0 {
            return None;
        }
        let pixel = |x: u32, y: u32| -> [u8; 4] {
            let i = (y as usize * width as usize + x as usize) * 4;
            rgba.get(i..i + 4)
                .and_then(|p| p.try_into().ok())
                .unwrap_or([0; 4])
        };
        if pixel(0, 1) != [255; 4] || pixel(7, 7)[3] != 0 {
            return None;
        }
        let (cell_w, cell_h) = (shape.v1 * 4, shape.v1 * 2);
        let (columns, rows) = (width / cell_w, height / cell_h);
        if columns < 2 || rows == 0 {
            return None;
        }
        // `info.rgb *= info.a * 255.` on normalized channels.
        let scaled = |p: [u8; 4]| p.map(|c| f32::from(c) * f32::from(p[3]) / 255.0);
        let armors = (1..columns)
            .filter_map(|column| {
                let x = cell_w * column;
                let [r, g, b, a] = pixel(x, 0);
                // The vertex colour it is compared with is opaque.
                if a != 255 {
                    return None;
                }
                let animation = scaled(pixel(x + 1, 0));
                let properties = scaled(pixel(x + 2, 0));
                Some(Armor {
                    key: u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
                    column,
                    frames: animation[0],
                    speed: animation[1],
                    tinted: properties[1].round() == 1.0,
                })
            })
            .collect();
        Some(Self {
            columns,
            rows,
            v2: shape.v2 as f32,
            armors,
        })
    }

    /// The part of the texture (u0, v0, u1, v1, all 0..1) a piece dyed `color` (RGB) shows at
    /// `day` (the shader's GameTime: the part of the day gone, 0..1), and whether the dye
    /// colours it.
    pub fn pick(&self, color: u32, day: f32) -> ([f32; 4], bool) {
        let (column, frame, tinted) = match self.armors.iter().find(|a| a.key == color) {
            Some(a) if a.frames >= 1.0 => {
                let timer = (day * self.v2 * a.speed).rem_euclid(a.frames).floor() as u32;
                (a.column, timer.min(self.rows - 1), a.tinted)
            }
            Some(a) => (a.column, 0, a.tinted),
            None => (0, 0, true),
        };
        let (w, h) = (1.0 / self.columns as f32, 1.0 / self.rows as f32);
        (
            [
                column as f32 * w,
                frame as f32 * h,
                (column + 1) as f32 * w,
                (frame + 1) as f32 * h,
            ],
            tinted,
        )
    }
}

/// The humanoid equipment layer textures under `root` (`entity/equipment/humanoid/leather`) a
/// pack's entity shader treats as custom armour: only with its constants in the shader, only
/// textures of the size they give.
pub fn load(root: &Path) -> HashMap<String, ArmorAtlas> {
    load_from(&horizon_assets::resource_source::ResourceSource::directory(
        root.to_owned(),
    ))
}

/// [`load`] from the resources `source` reads: a resource reload's, with the packs over them.
pub fn load_from(
    source: &horizon_assets::resource_source::ResourceSource,
) -> HashMap<String, ArmorAtlas> {
    let mut atlases = HashMap::new();
    let Some(shape) = ["entity.fsh", "rendertype_armor_cutout_no_cull.fsh"]
        .iter()
        .find_map(|f| {
            let text = source
                .read_to_string(&format!("assets/minecraft/shaders/core/{f}"))
                .ok()?;
            AtlasShape::parse(&text)
        })
    else {
        return atlases;
    };
    const EQUIPMENT: &str = "assets/minecraft/equipment/";
    for path in source.list(EQUIPMENT) {
        // The folder's own files, as vanilla's equipment assets are.
        let Some(name) = (path.strip_prefix(EQUIPMENT))
            .and_then(|n| n.strip_suffix(".json"))
            .filter(|n| !n.contains('/'))
        else {
            continue;
        };
        let Some(asset) = (source.read(&path).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        else {
            continue;
        };
        for kind in ["humanoid", "humanoid_leggings"] {
            let Some(layer) = asset["layers"][kind].get(0) else {
                continue;
            };
            let texture = layer["texture"]
                .as_str()
                .unwrap_or(name)
                .trim_start_matches("minecraft:");
            let texture = format!("entity/equipment/{kind}/{texture}");
            if atlases.contains_key(&texture) {
                continue;
            }
            let file = format!("assets/minecraft/textures/{texture}.png");
            // The PNG header's size first: only a texture as big as the shader says is decoded.
            let Some(header) = source.read_head(&file, 24).ok().filter(|h| h.len() >= 24) else {
                continue;
            };
            let be = |i: usize| {
                u32::from_be_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]])
            };
            if (be(16), be(20)) != (shape.width, shape.height) {
                continue;
            }
            if let Some((w, h, rgba)) =
                (source.read(&file).ok()).and_then(|bytes| crate::hud::decode_bytes(&bytes).ok())
                && let Some(atlas) = ArmorAtlas::read(shape, w, h, &rgba)
            {
                log::info!("Horizon: {texture} is a pack's custom armour, picked by dye colour");
                atlases.insert(texture, atlas);
            }
        }
    }
    atlases
}

/// How an equipment layer is drawn: the part of its sprite (`uv`) and the colour (ABGR), for a
/// layer coloured `rgb` (a dyeable one: its dye colour, else the undyed one; none for the
/// others), with the custom armour of that colour when its texture is an `atlas`.
pub fn layer_look(
    atlas: Option<&ArmorAtlas>,
    rgb: Option<u32>,
    uv: [f32; 4],
    day: f32,
) -> ([f32; 4], u32) {
    let mut uv = uv;
    let mut tinted = true;
    if let (Some(atlas), Some(rgb)) = (atlas, rgb) {
        let ([a, b, c, d], dyed) = atlas.pick(rgb, day);
        let (du, dv) = (uv[2] - uv[0], uv[3] - uv[1]);
        uv = [
            uv[0] + du * a,
            uv[1] + dv * b,
            uv[0] + du * c,
            uv[1] + dv * d,
        ];
        tinted = dyed;
    }
    let color = rgb.filter(|_| tinted).map_or(0xFFFF_FFFF, |c| {
        0xFF00_0000 | (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF
    });
    (uv, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHADER: &str = "//%START_CUSTOM_ARMORS_VARS%\n#define V1 4\n#define V2 50\n\
                          #define TEX_WIDTH 48\n#define TEX_HEIGHT 16\n//%END_CUSTOM_ARMORS_VARS%";

    /// Three columns of 16 by 8, two frames: the leather, a still armour keyed #A06540 dyed
    /// as well, an armour keyed #28A7F1 of two frames at speed 1.
    fn atlas() -> ArmorAtlas {
        let shape = AtlasShape::parse(SHADER).unwrap();
        assert_eq!(
            shape,
            AtlasShape {
                v1: 4,
                v2: 50,
                width: 48,
                height: 16
            }
        );
        let mut rgba = vec![0u8; 48 * 16 * 4];
        let mut set = |x: usize, y: usize, p: [u8; 4]| {
            rgba[(y * 48 + x) * 4..][..4].copy_from_slice(&p);
        };
        set(0, 1, [255; 4]);
        set(16, 0, [0xA0, 0x65, 0x40, 255]);
        set(18, 0, [0, 1, 0, 255]);
        set(32, 0, [0x28, 0xA7, 0xF1, 255]);
        set(33, 0, [2, 1, 0, 255]);
        ArmorAtlas::read(shape, 48, 16, &rgba).unwrap()
    }

    #[test]
    fn a_dye_colour_picks_its_armour_and_frame() {
        let atlas = atlas();
        let third = 1.0 / 3.0;
        // Any other colour: the leather, dyed.
        assert_eq!(atlas.pick(0x123456, 0.0), ([0.0, 0.0, third, 0.5], true));
        assert_eq!(
            atlas.pick(0xA06540, 0.0),
            ([third, 0.0, 2.0 * third, 0.5], true)
        );
        // 50 frame steps a day at speed 1: the second frame from 1/50 of the day on.
        let (uv, tinted) = atlas.pick(0x28A7F1, 0.03);
        assert_eq!((uv[1], uv[3], tinted), (0.5, 1.0, false));
        assert_eq!(atlas.pick(0x28A7F1, 0.01).0[1], 0.0);
    }

    #[test]
    fn a_layer_shows_its_armour_in_the_sprite_and_drops_the_dye_unless_asked() {
        let atlas = atlas();
        let sprite = [0.5, 0.25, 0.8, 0.45];
        // Plain leather: the first column, tinted (ABGR).
        let (uv, color) = layer_look(Some(&atlas), Some(0x123456), sprite, 0.0);
        assert!((uv[2] - 0.6).abs() < 1e-6 && (uv[3] - 0.35).abs() < 1e-6);
        assert_eq!(color, 0xFF56_3412);
        // A custom armour that is not dyed: white.
        assert_eq!(
            layer_look(Some(&atlas), Some(0x28A7F1), sprite, 0.0).1,
            0xFFFF_FFFF
        );
        // Not an atlas: the whole sprite.
        assert_eq!(layer_look(None, None, sprite, 0.0), (sprite, 0xFFFF_FFFF));
    }

    #[test]
    fn a_texture_of_another_size_or_without_the_mark_is_no_atlas() {
        let shape = AtlasShape::parse(SHADER).unwrap();
        assert!(ArmorAtlas::read(shape, 64, 32, &vec![255; 64 * 32 * 4]).is_none());
        assert!(ArmorAtlas::read(shape, 48, 16, &vec![0; 48 * 16 * 4]).is_none());
        assert!(AtlasShape::parse("#define V1 64").is_none());
    }
}
