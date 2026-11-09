//! Voice Chat's status icons, in every colour: 16x16 PNGs in data/icons,
//! built into the game, decoded once.
use std::collections::HashMap;
use std::sync::OnceLock;

/// The colours, in the order the settings cycle through them.
pub const COLOURS: [&str; 6] = ["blue", "grey", "black", "white", "red", "rainbow"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Idle,
    Talking,
    Muted,
    Disabled,
    Connecting,
    /// Someone else talking.
    Speaker,
    Group,
}

pub const ICONS: [Icon; 7] = [
    Icon::Idle,
    Icon::Talking,
    Icon::Muted,
    Icon::Disabled,
    Icon::Connecting,
    Icon::Speaker,
    Icon::Group,
];

/// Pixels as 0xAARRGGBB, row by row.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub argb: Vec<u32>,
}

macro_rules! colour {
    ($c:literal) => {
        [
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/data/icons/",
                $c,
                "/idle.png"
            ))
            .as_slice(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/data/icons/",
                $c,
                "/talking.png"
            ))
            .as_slice(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/data/icons/",
                $c,
                "/muted.png"
            ))
            .as_slice(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/data/icons/",
                $c,
                "/disabled.png"
            ))
            .as_slice(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/data/icons/",
                $c,
                "/connecting.png"
            ))
            .as_slice(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/data/icons/",
                $c,
                "/speaker.png"
            ))
            .as_slice(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/data/icons/",
                $c,
                "/group.png"
            ))
            .as_slice(),
        ]
    };
}

const FILES: [[&[u8]; 7]; 6] = [
    colour!("blue"),
    colour!("grey"),
    colour!("black"),
    colour!("white"),
    colour!("red"),
    colour!("rainbow"),
];

fn decode(bytes: &[u8]) -> Option<Image> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let px = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        _ => return None,
    };
    let argb = buf[..info.buffer_size()]
        .chunks_exact(px)
        .map(|p| {
            let (r, g, b, a) = match px {
                4 => (p[0], p[1], p[2], p[3]),
                3 => (p[0], p[1], p[2], 255),
                2 => (p[0], p[0], p[0], p[1]),
                _ => (p[0], p[0], p[0], 255),
            };
            (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
        })
        .collect();
    Some(Image {
        width: info.width,
        height: info.height,
        argb,
    })
}

/// The icon `icon` in the colour `colour` (an unknown colour is blue).
pub fn icon(colour: &str, icon: Icon) -> Option<&'static Image> {
    static ALL: OnceLock<HashMap<(usize, Icon), Image>> = OnceLock::new();
    let all = ALL.get_or_init(|| {
        let mut all = HashMap::new();
        for (c, files) in FILES.iter().enumerate() {
            for (i, bytes) in files.iter().enumerate() {
                match decode(bytes) {
                    Some(img) => {
                        all.insert((c, ICONS[i]), img);
                    }
                    None => log::warn!(
                        "Voice chat: icon {}/{:?} didn't decode",
                        COLOURS[c],
                        ICONS[i]
                    ),
                }
            }
        }
        all
    });
    let c = COLOURS.iter().position(|n| *n == colour).unwrap_or(0);
    all.get(&(c, icon))
}

/// Horizontal runs of one colour: (x, y, length, argb), transparent
/// pixels left out. Drawing a run is one rectangle.
pub fn runs(img: &Image) -> Vec<(u32, u32, u32, u32)> {
    let mut out = Vec::new();
    for y in 0..img.height {
        let row = &img.argb[(y * img.width) as usize..((y + 1) * img.width) as usize];
        let mut x = 0;
        while x < img.width {
            let c = row[x as usize];
            let mut end = x + 1;
            while end < img.width && row[end as usize] == c {
                end += 1;
            }
            if c >> 24 != 0 {
                out.push((x, y, end - x, c));
            }
            x = end;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_decodes() {
        for c in COLOURS {
            for i in ICONS {
                let img = icon(c, i).unwrap_or_else(|| panic!("{c}/{i:?}"));
                assert_eq!((img.width, img.height), (16, 16));
                assert!(
                    img.argb.iter().any(|p| p >> 24 == 255),
                    "{c}/{i:?} is empty"
                );
                assert!(!runs(img).is_empty());
            }
        }
        assert!(
            icon("purple", Icon::Idle).is_some(),
            "unknown colours fall back to blue"
        );
    }
}
