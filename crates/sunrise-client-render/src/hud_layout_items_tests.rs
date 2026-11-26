//! Engine draw-list painting: items squeeze about their pivots without squeezing decorations,
//! blits retain their blend, and GUI, model, live and registered atlas lookups stay separate.
use super::super::{Blend, GuiDraw, PoseOp, paint};
use super::{GuiItems, paint_items};
use crate::hud::{Blend as Run, HudAtlas, HudBatch};

/// A run-time atlas slot every atlas has: map slot 0 (`hud::map_slot_sprite`).
const SLOT: &str = "sunrise/map0";

fn atlas() -> HudAtlas {
    HudAtlas::build(&crate::resource_source::ResourceSource::directory(
        "/nonexistent/sunrise-test-assets".into(),
    ))
    .unwrap()
}

/// Draws each slot's item as a 16 x 16 square in colour `ITEM + slot` and its decorations as a
/// 4 x 4 square in `DECORATION + slot` at the bottom right, recording each call.
#[derive(Default)]
struct Items {
    calls: Vec<(&'static str, u32, i32, i32, i32)>,
}
const ITEM: u32 = 0xFF00_0000;
const DECORATION: u32 = 0xFF10_0000;
impl GuiItems for Items {
    fn item(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32, seed: i32) {
        self.calls.push(("item", slot, x, y, seed));
        let atlas = atlas();
        batch.fill(&atlas, x as f32, y as f32, 16.0, 16.0, ITEM + slot);
    }
    fn decorations(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32) {
        self.calls.push(("decorations", slot, x, y, 0));
        let atlas = atlas();
        batch.fill(
            &atlas,
            (x + 12) as f32,
            (y + 12) as f32,
            4.0,
            4.0,
            DECORATION + slot,
        );
    }
}

fn item(slot: u32, x: i32, y: i32, scale: [f32; 2], pivot: [f32; 2]) -> GuiDraw {
    GuiDraw::Item {
        slot,
        x,
        y,
        seed: 7,
        scale,
        pivot,
    }
}

fn sprite(name: &'static str, region: Option<[i32; 4]>, blend: Blend) -> GuiDraw {
    GuiDraw::Sprite {
        sprite: name.into(),
        x: 0,
        y: 0,
        w: 4,
        h: 4,
        region,
        color: 0xFFFF_FFFF,
        blend,
    }
}

fn blit(texture: &'static str, blend: Blend, tex: [i32; 2]) -> GuiDraw {
    GuiDraw::Blit {
        texture: texture.into(),
        blend,
        x: 0,
        y: 0,
        w: 8,
        h: 8,
        uv: [0.0, 0.0],
        src: [8, 8],
        tex,
        color: 0xFFFF_FFFF,
    }
}

/// The smallest and largest corner of the vertices in `color`.
fn bounds(batch: &HudBatch, color: u32) -> [f32; 4] {
    let mut b = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for v in batch.vertices.iter().filter(|v| v.color == color) {
        b = [
            b[0].min(v.pos[0]),
            b[1].min(v.pos[1]),
            b[2].max(v.pos[0]),
            b[3].max(v.pos[1]),
        ];
    }
    b
}

#[test]
fn an_item_is_scaled_about_its_pivot_and_its_decorations_are_not() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    let mut items = Items::default();
    let draws = [item(0, 20, 20, [2.0, 2.0], [28.0, 28.0])];
    paint_items(&mut batch, &atlas, &draws, &mut items);
    assert_eq!(
        items.calls,
        [("item", 0, 20, 20, 7), ("decorations", 0, 20, 20, 0)]
    );
    assert_eq!(bounds(&batch, ITEM), [12.0, 12.0, 44.0, 44.0]);
    assert_eq!(bounds(&batch, DECORATION), [32.0, 32.0, 36.0, 36.0]);
}

#[test]
fn items_follow_the_current_pose_and_every_hotbar_slot_and_the_off_hand_draw() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    let mut items = Items::default();
    let draws = [
        GuiDraw::Push(vec![PoseOp::Translate(100.0, 10.0)]),
        item(8, 0, 0, [1.0, 1.0], [8.0, 12.0]),
        GuiDraw::Pop,
        item(40, 0, 0, [1.0, 1.0], [8.0, 12.0]),
    ];
    paint_items(&mut batch, &atlas, &draws, &mut items);
    assert_eq!(bounds(&batch, ITEM + 8), [100.0, 10.0, 116.0, 26.0]);
    assert_eq!(bounds(&batch, DECORATION + 8), [112.0, 22.0, 116.0, 26.0]);
    assert_eq!(bounds(&batch, ITEM + 40), [0.0, 0.0, 16.0, 16.0]);
}

#[test]
fn every_blit_blend_draws_with_its_blend() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    let draws = [
        blit(SLOT, Blend::Textured, [8, 8]),
        blit(SLOT, Blend::Vignette, [8, 8]),
        blit(SLOT, Blend::Nausea, [8, 8]),
        blit(SLOT, Blend::Crosshair, [8, 8]),
    ];
    paint_items(&mut batch, &atlas, &draws, &mut Items::default());
    assert_eq!(batch.vertices.len(), 24);
    assert_eq!(
        batch.runs(),
        [
            (0..6, Run::Alpha),
            (6..12, Run::Vignette),
            (12..18, Run::Additive),
            (18..24, Run::Invert),
        ]
    );
}

#[test]
fn paints_gui_packed_and_registered_atlas_sprites_from_separate_lookups() {
    let root = std::env::temp_dir().join(format!("sunrise-layout-atlases-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let sheet = root.join("assets/demo/textures/item");
    let atlases = root.join("assets/minecraft/atlases");
    std::fs::create_dir_all(&sheet).unwrap();
    std::fs::create_dir_all(&atlases).unwrap();
    let mut encoder = png::Encoder::new(
        std::fs::File::create(sheet.join("sheet.png")).unwrap(),
        1,
        1,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[1, 2, 3, 255])
        .unwrap();
    let staged = root.join("assets/demo/textures/gui/atlases/icons/demo");
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::copy(sheet.join("sheet.png"), staged.join("heart.png")).unwrap();
    std::fs::write(atlases.join("blocks.json"), r#"{"sources":[]}"#).unwrap();
    std::fs::write(
        atlases.join("items.json"),
        r#"{"sources":[{"type":"minecraft:single","resource":"demo:item/sheet","sprite":"demo:item/modeled"}]}"#,
    )
    .unwrap();
    std::fs::write(
        atlases.join("gui.json"),
        r#"{"sources":[
            {"type":"minecraft:single","resource":"demo:item/sheet","sprite":"demo:kept"},
            {"type":"minecraft:single","resource":"demo:item/sheet","sprite":"demo:item/modeled"},
            {"type":"minecraft:filter","pattern":{"namespace":"demo","path":"item/modeled"}}]}"#,
    )
    .unwrap();
    let atlas = HudAtlas::build(&crate::resource_source::ResourceSource::directory(
        root.clone(),
    ))
    .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let mut gui = HudBatch::scratch([64, 64]);
    paint(
        &mut gui,
        &atlas,
        &[sprite("demo:kept", None, Blend::Textured)],
    );
    assert_eq!(gui.vertices.len(), 6);
    let block = GuiDraw::AtlasSprite {
        sprite: "demo:item/modeled".into(),
        x: 0,
        y: 0,
        w: 4,
        h: 4,
        color: u32::MAX,
    };
    let mut batch = HudBatch::scratch([64, 64]);
    paint(&mut batch, &atlas, std::slice::from_ref(&block));
    let model = atlas.sprites["demo:item/modeled"];
    assert_eq!(
        batch.vertices[0].uv,
        [model.u0, model.v0],
        "the block atlas's sprite"
    );
    // A registered atlas's sprite no packed sprite shares its name with draws as a `Blit`.
    let name = "demo:gui/atlases/icons/demo/heart";
    let registered = atlas.registered[name];
    let heart = blit(name, Blend::Textured, [8, 8]);
    let mut batch = HudBatch::scratch([64, 64]);
    paint(&mut batch, &atlas, &[heart]);
    assert_eq!(batch.vertices[0].uv, [registered.u0, registered.v0]);
}

#[test]
fn plain_paint_still_leaves_items_and_screen_effect_blits_to_its_caller() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    let draws = [
        item(0, 0, 0, [1.0, 1.0], [0.0, 0.0]),
        blit(SLOT, Blend::Vignette, [8, 8]),
        blit(SLOT, Blend::Nausea, [8, 8]),
        blit(SLOT, Blend::Crosshair, [8, 8]),
    ];
    paint(&mut batch, &atlas, &draws);
    assert!(batch.vertices.is_empty());
    // The hotbar's path draws them.
    let mut items = Items::default();
    paint_items(&mut batch, &atlas, &draws, &mut items);
    assert_eq!(items.calls.len(), 2);
    assert_eq!(batch.vertices.len(), 6 * 3 + 6 * 2);
}
