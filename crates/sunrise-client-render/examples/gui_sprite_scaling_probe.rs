//! The Rust side of tools/test_gui_sprite_scaling_parity.py: GUI sprite metadata through the loader, and tiled copies.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};
use sunrise_assets::resource_source::{Resource, ResourceSource};
use sunrise_client_render::{
    hud::{HudBatch, Rect},
    ui::{self, Scaling, SpriteMetadata},
};

const SPRITE: &str = "assets/minecraft/textures/gui/sprites/probe.png";
const MCMETA: &str = "assets/minecraft/textures/gui/sprites/probe.png.mcmeta";

fn decode(row: &Value) -> Value {
    let text = row["json"].as_str().unwrap().as_bytes().to_vec();
    let source =
        ResourceSource::indexed(BTreeMap::from([(MCMETA.to_owned(), Resource::bytes(text))]));
    match ui::sprite_metadata_of(&source, SPRITE) {
        None => json!({"type": "stretch"}),
        Some(SpriteMetadata::Rejected) => json!({"error": true}),
        Some(SpriteMetadata::Scaled(Scaling::Tile { width, height })) => {
            json!({"type": "tile", "width": width, "height": height})
        }
        Some(SpriteMetadata::Scaled(Scaling::NineSlice(n))) => json!({
            "type": "nine_slice", "width": n.width, "height": n.height,
            "border": n.border, "stretch_inner": n.stretch_inner,
        }),
    }
}

fn int(row: &Value, key: &str) -> i64 {
    row[key].as_i64().unwrap()
}

fn tile(row: &Value) -> Value {
    // `TextureAtlasSprite`'s UVs: (float)(x + padding) / atlasWidth, padding 0.
    let (atlas_w, atlas_h) = (int(row, "atlas_w") as f32, int(row, "atlas_h") as f32);
    let (x, y, cw, ch) = (
        int(row, "x"),
        int(row, "y"),
        int(row, "contents_w"),
        int(row, "contents_h"),
    );
    let rect = Rect {
        u0: x as f32 / atlas_w,
        u1: (x + cw) as f32 / atlas_w,
        v0: y as f32 / atlas_h,
        v1: (y + ch) as f32 / atlas_h,
        w: cw as u32,
        h: ch as u32,
    };
    let f = |key: &str| int(row, key) as f32;
    let mut batch = HudBatch::scratch([4096, 4096]);
    ui::tile(
        &mut batch,
        rect,
        [f("sprite_w"), f("sprite_h")],
        [f("texture_x"), f("texture_y"), f("tile_w"), f("tile_h")],
        [f("x0"), f("y0"), f("w"), f("h")],
        u32::MAX,
    );
    let quads: Vec<Vec<u32>> = batch
        .vertices
        .chunks(6)
        .map(|q| {
            let (a, c) = (q[0], q[2]);
            [
                a.pos[0], a.pos[1], c.pos[0], c.pos[1], a.uv[0], a.uv[1], c.uv[0], c.uv[1],
            ]
            .map(f32::to_bits)
            .to_vec()
        })
        .collect();
    json!({"quads": quads})
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut out = BufWriter::new(File::create(&args[2])?);
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let value = match row["kind"].as_str().unwrap() {
            "decode" => decode(&row),
            _ => tile(&row),
        };
        writeln!(out, "{}", json!({"name": row["name"], "value": value}))?;
    }
    Ok(())
}
