//! HUD boxes in the corners: mods' panels, small boxes in GUI pixels stacked per corner
//! (`docs/extras.md`), and the layers crates above the engine register (`sunrise_hooks::hud`).
use crate::{
    extras::Corner,
    hud::{HudAtlas, HudBatch, rgba},
};
pub use mod_api::client::extras_hud::Corners;
use sunrise_hooks::hud;

const BACKGROUND: u32 = 0x7800_0000;

/// A box of text lines (a mod's HUD panel): `(text, 0xRRGGBB)` each.
pub fn draw_lines(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    corners: &mut Corners,
    corner: Corner,
    lines: &[(&str, u32)],
) {
    if lines.is_empty() {
        return;
    }
    let w = lines
        .iter()
        .map(|(text, _)| HudBatch::text_width(atlas, text))
        .fold(0.0, f32::max)
        + 4.0;
    let h = 10.0 * lines.len() as f32 + 2.0;
    let (x, y) = corners.place(corner, w, h);
    batch.fill(atlas, x, y, w, h, BACKGROUND);
    for (i, (text, color)) in lines.iter().enumerate() {
        let [_, r, g, b] = color.to_be_bytes();
        let y = y + 2.0 + 10.0 * i as f32;
        batch.text(atlas, text, x + 2.0, y, rgba(r, g, b, 255), true);
    }
}

/// Draws the HUD layers crates registered with the HUD ([`hud::Stage::Hud`]), after the extras'
/// and mods' boxes, which they stack with. `shown`: whether each corner of `Corner::ALL` shows
/// (the debug screen covers some).
pub fn draw_layers(
    layers: &[&dyn hud::Layer],
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    corners: &mut Corners,
    shown: [bool; 4],
) {
    let mut canvas = Canvas {
        batch,
        atlas,
        corners,
        shown,
        // The HUD's own layers draw only in play.
        over_menu: false,
    };
    for layer in layers.iter().filter(|l| l.stage() == hud::Stage::Hud) {
        layer.draw(&mut canvas);
    }
}

/// Draws the layers crates registered over everything ([`hud::Stage::Top`]), last. Nothing is
/// stacked in a corner there: every corner is hidden. With none, no canvas.
pub fn draw_top_layers(
    layers: &[&dyn hud::Layer],
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    over_menu: bool,
) {
    let mut top = layers
        .iter()
        .filter(|l| l.stage() == hud::Stage::Top)
        .peekable();
    if top.peek().is_none() {
        return;
    }
    let mut corners = Corners::new(batch.width, batch.height, 0.0);
    let mut canvas = Canvas {
        batch,
        atlas,
        corners: &mut corners,
        shown: [false; 4],
        over_menu,
    };
    for layer in top {
        layer.draw(&mut canvas);
    }
}

struct Canvas<'a> {
    batch: &'a mut HudBatch,
    atlas: &'a HudAtlas,
    corners: &'a mut Corners,
    shown: [bool; 4],
    /// Starling: a menu is under the layers (`hud::Canvas::over_menu`).
    over_menu: bool,
}

/// `0xAARRGGBB` as the batch's colour.
fn argb_color(argb: u32) -> u32 {
    let [a, r, g, b] = argb.to_be_bytes();
    rgba(r, g, b, a)
}

impl hud::Canvas for Canvas<'_> {
    fn size(&self) -> (f32, f32) {
        (self.batch.width, self.batch.height)
    }
    fn tr(&self, key: &str) -> String {
        crate::lang::get(key)
    }
    fn text_width(&self, text: &str) -> f32 {
        HudBatch::text_width(self.atlas, text)
    }
    fn text(&mut self, x: f32, y: f32, text: &str, argb: u32, shadow: bool) {
        self.batch
            .text(self.atlas, text, x, y, argb_color(argb), shadow);
    }
    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, argb: u32) {
        self.batch.fill(self.atlas, x, y, w, h, argb_color(argb));
    }
    fn place(&mut self, corner: hud::Corner, w: f32, h: f32) -> Option<(f32, f32)> {
        let corner = match corner {
            hud::Corner::TopLeft => Corner::TopLeft,
            hud::Corner::TopRight => Corner::TopRight,
            hud::Corner::BottomLeft => Corner::BottomLeft,
            hud::Corner::BottomRight => Corner::BottomRight,
        };
        let i = Corner::ALL.iter().position(|c| *c == corner)?;
        self.shown[i].then(|| self.corners.place(corner, w, h))
    }
    fn over_menu(&self) -> bool {
        self.over_menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_stack_away_from_their_edge() {
        let mut c = Corners::new(400.0, 300.0, 26.0);
        assert_eq!(c.place(Corner::TopLeft, 50.0, 12.0), (2.0, 2.0));
        assert_eq!(c.place(Corner::TopLeft, 70.0, 20.0), (2.0, 16.0));
        // Top right starts under the effect icons.
        assert_eq!(c.place(Corner::TopRight, 50.0, 12.0), (348.0, 28.0));
        assert_eq!(c.place(Corner::BottomRight, 50.0, 12.0), (348.0, 286.0));
        assert_eq!(c.place(Corner::BottomRight, 50.0, 10.0), (348.0, 274.0));
        assert_eq!(c.place(Corner::BottomLeft, 10.0, 10.0), (2.0, 288.0));
    }

    /// Draws a box of one line in a corner and keeps where it went.
    struct Panel {
        corner: hud::Corner,
        at: std::sync::Mutex<Vec<Option<(f32, f32)>>>,
    }

    impl hud::Layer for Panel {
        fn draw(&self, canvas: &mut dyn hud::Canvas) {
            let w = canvas.text_width("12 CPS") + 4.0;
            let at = canvas.place(self.corner, w, 12.0);
            if let Some((x, y)) = at {
                canvas.fill(x, y, w, 12.0, 0x7800_0000);
                canvas.text(x + 2.0, y + 2.0, "12 CPS", 0xFFFF_FFFF, true);
            }
            self.at.lock().unwrap().push(at);
        }
    }

    #[test]
    fn layers_stack_with_the_extras_and_skip_hidden_corners() {
        let atlas = HudAtlas::build(&crate::resource_source::ResourceSource::indexed(
            Default::default(),
        ))
        .unwrap();
        let mut batch = HudBatch::new(
            [800, 600],
            crate::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        );
        let (width, height) = (batch.width, batch.height);
        let mut corners = Corners::new(width, height, 0.0);
        // Another box already took the top right.
        corners.place(Corner::TopRight, 64.0, 64.0);
        let right = Panel {
            corner: hud::Corner::TopRight,
            at: Default::default(),
        };
        let left = Panel {
            corner: hud::Corner::TopLeft,
            at: Default::default(),
        };
        let layers: [&dyn hud::Layer; 2] = [&right, &left];
        let empty = batch.vertices.len();
        draw_layers(
            &layers,
            &mut batch,
            &atlas,
            &mut corners,
            [false, true, true, true],
        );
        let (_, y) = right.at.lock().unwrap()[0].expect("the top right shows");
        assert_eq!(y, 2.0 + 64.0 + 2.0, "under the other box");
        assert_eq!(
            *left.at.lock().unwrap(),
            [None],
            "the debug screen covers the top left"
        );
        assert!(batch.vertices.len() > empty, "the shown box is drawn");
    }
}
