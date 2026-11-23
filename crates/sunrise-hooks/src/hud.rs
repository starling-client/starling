//! HUD drawing (docs/layers.md, "Platform registries"): a crate above the engine draws on the HUD
//! where the extras do, in the corners they share, or over everything, without an engine file
//! naming it. With no layer registered the engine builds no canvas.
use crate::TooLate;
use crate::registry::Registry;

/// A corner of the screen. Boxes placed in one stack away from its edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// What a layer draws with, in GUI pixels (the player's GUI scale applied). Colours are
/// `0xAARRGGBB`.
pub trait Canvas {
    /// The screen's width and height.
    fn size(&self) -> (f32, f32);
    /// The text of a translation key in the player's language.
    fn tr(&self, key: &str) -> String;
    /// How wide `text` is drawn.
    fn text_width(&self, text: &str) -> f32;
    /// A line of text from its top-left corner, with the game's text shadow or not.
    fn text(&mut self, x: f32, y: f32, text: &str, argb: u32, shadow: bool);
    /// A filled rectangle.
    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, argb: u32);
    /// Where a `w` × `h` box goes in `corner`: below (or above) what the corner holds already,
    /// then any earlier layer in that corner. `None` while the corner is hidden (the debug
    /// screen covers the top left): draw nothing there then.
    fn place(&mut self, corner: Corner, w: f32, h: f32) -> Option<(f32, f32)>;
    /// Starling: whether a menu (the title, the options, a screen over the world) is under
    /// this [`Stage::Top`] layer, where a layer may draw less so it keeps off the menu's buttons.
    fn over_menu(&self) -> bool {
        false
    }
}

/// When a [`Layer`] draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// With the HUD, every frame it is drawn (not with F1, nor under a screen that hides it),
    /// stacked around occupied corners.
    #[default]
    Hud,
    /// After the debug screen and before foreground GUI overlays such as input feedback. Drawn
    /// over menus, toasts and the title screen, F1 included. Nothing is stacked there:
    /// [`Canvas::place`] is `None`.
    Top,
}

/// Something a crate draws on the HUD. It reads its own state and must return quickly.
pub trait Layer: Sync {
    fn draw(&self, canvas: &mut dyn Canvas);
    /// When it draws: with the HUD unless it says otherwise.
    fn stage(&self) -> Stage {
        Stage::Hud
    }
}

static LAYERS: Registry<dyn Layer> = Registry::new();

/// Adds `layer` to the HUD, after the layers registered before it in the same stage. Before the game starts, as
/// [`crate::register`].
pub fn register(layer: &'static dyn Layer) -> Result<(), TooLate> {
    LAYERS.register(layer)
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    use super::*;

    /// Every registered layer, sealing the list on the first call.
    pub fn all() -> &'static [&'static dyn Layer] {
        LAYERS.all()
    }
}
