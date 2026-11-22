#![cfg(test)]
//! `DrawContext::gui_paint` through the game's generated dispatch (#4527): an `Item` draws the
//! local player's hotbar stack scaled about its pivot and its decorations, a `Blit` of a screen
//! effect blend draws with that blend, and an op the paint cannot draw refuses the buffer with
//! its name and draws nothing of it.
use mod_api::client::hud_layout::{Blend, GuiDraw};
use mod_api::{ComponentHoverEvent, GuiPaintError, Hooks, ModEntry};
use mod_platform::testing::with_mods;
use std::sync::Mutex;
use sunrise_client::hud::{Blend as Run, HudBatch, map_slot_sprite};
use sunrise_client::items::ItemStack;
use sunrise_client::text::{Style, Text};

/// What each `gui_paint` answered, in order.
static ANSWERS: Mutex<Vec<Result<(), GuiPaintError>>> = Mutex::new(Vec::new());

fn item(slot: u32, scale: [f32; 2]) -> GuiDraw {
    GuiDraw::Item {
        slot,
        x: 20,
        y: 20,
        seed: 0,
        scale,
        pivot: [28.0, 28.0],
    }
}

fn vignette() -> GuiDraw {
    GuiDraw::Blit {
        texture: map_slot_sprite(0).unwrap().into(),
        blend: Blend::Vignette,
        x: 100,
        y: 0,
        w: 8,
        h: 8,
        uv: [0.0, 0.0],
        src: [8, 8],
        tex: [8, 8],
        color: 0xFFFF_FFFF,
    }
}

/// Paints the issue's repro, then a buffer with an off-hotbar slot after a drawable op.
struct Painter;
#[mod_api::__subscriptions]
impl Hooks for Painter {
    fn component_hover(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut ComponentHoverEvent<'_>,
    ) -> mod_api::Flow {
        let mut answers = ANSWERS.lock().unwrap();
        answers.push(event.draw.gui_paint(&[item(0, [2.0, 2.0]), vignette()]));
        answers.push(event.draw.gui_paint(&[vignette(), item(12, [1.0, 1.0])]));
        mod_api::Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn a_mods_paint_draws_hotbar_items_and_blits_and_refuses_what_it_cannot_draw() {
    let text = Text::from(serde_json::json!({
        "text": "x", "hover_event": { "action": "show_text", "value": "hi" }
    }));
    let style: Style = text.spans[0].style.clone();
    let mods: mod_api::Mods = vec![ModEntry::new("painter", Box::new(Painter))].into();
    let (mut scaled, mut plain) = (HudBatch::scratch([320, 240]), HudBatch::scratch([320, 240]));
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.inventory[0] = Some(ItemStack::new("minecraft:stone", 5));
            game.component_hover_effect(&mut scaled, &style, (40.0, 30.0));
            // The same stack as `DrawContext::item_stack` would draw it, unscaled, for comparison.
            game.draw_item_model(
                &mut plain,
                &ItemStack::new("minecraft:stone", 5),
                20.0,
                20.0,
            );
        });
    });
    let answers = std::mem::take(&mut *ANSWERS.lock().unwrap());
    assert_eq!(
        answers,
        [Ok(()), Err(GuiPaintError::ItemSlot { index: 1, slot: 12 })]
    );
    // The model's vertices, each moved twice as far from the pivot (28, 28).
    assert!(!plain.vertices.is_empty(), "no stone model drawn");
    for (s, p) in scaled.vertices.iter().zip(&plain.vertices) {
        let want = p.pos.map(|v| 28.0 + (v - 28.0) * 2.0);
        assert!(
            (s.pos[0] - want[0]).abs() < 1e-3 && (s.pos[1] - want[1]).abs() < 1e-3,
            "{:?} is not {want:?}",
            s.pos
        );
    }
    // Then the count (5) unscaled, then the vignette with its blend; the refused buffer drew
    // nothing after that.
    let runs = scaled.runs();
    let (last, blend) = runs.last().unwrap().clone();
    assert_eq!(blend, Run::Vignette);
    assert_eq!(last.end as usize, scaled.vertices.len());
    assert_eq!(last.len(), 6);
    assert!(
        scaled.vertices.len() > plain.vertices.len() + 6,
        "no decorations"
    );
}
