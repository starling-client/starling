//! The settings screens' look: vanilla's widget sprites and colours, and the shell's sizes.
use crate::menu_style::{self, NEUTRAL, SettingsColours};
use crate::{
    hud::{HudAtlas, HudBatch},
    text::Text,
    ui,
};

/// Vanilla's greys and chat colours (`menu_style::NEUTRAL`).
fn b() -> &'static SettingsColours {
    &NEUTRAL.settings
}

/// What a setting costs in frame time, shown with its description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cost {
    None,
    Low,
    Medium,
    High,
    /// Turning it on makes frames faster (culling, caching, batching).
    Saves,
}
impl Cost {
    pub fn label(self) -> &'static str {
        match self {
            Cost::None => "No cost",
            Cost::Low => "Low cost",
            Cost::Medium => "Medium cost",
            Cost::High => "High cost",
            Cost::Saves => "Saves time",
        }
    }
    /// `0xRRGGBB`: vanilla's green, yellow and red.
    pub fn rgb(self) -> u32 {
        match self {
            Cost::None => b().ink_soft,
            Cost::Low | Cost::Saves => b().success,
            Cost::Medium => b().warn,
            Cost::High => b().danger,
        }
    }
}

use menu_style::hud_colour as hex;
/// A hovered or focused row.
pub fn hover() -> u32 {
    hex(b().ink, 18)
}
/// The open page in the page list.
pub fn open() -> u32 {
    hex(b().ink, 26)
}
/// The rule under the plate's heading, and between two rows of a group.
pub fn head_rule() -> u32 {
    hex(b().ink, 31)
}
pub fn row_rule() -> u32 {
    hex(b().ink, 15)
}
pub fn accent() -> u32 {
    hex(b().accent, 255)
}
/// Vanilla's white text, and its grey for what can't be used now.
pub const WHITE: u32 = u32::MAX;
pub fn muted() -> u32 {
    hex(b().muted, 255)
}
/// A page that isn't open, a value at rest.
pub fn soft() -> u32 {
    hex(b().ink_soft, 255)
}
/// Vanilla's grey for a tooltip's lines under its first (`ChatFormatting.GRAY`).
pub const GREY: u32 = hex(0xaaaaaa, 255);

/// Sizes in GUI pixels.
/// Shorter screens than this (a 720p window at scale 3) keep a narrower margin on top.
pub const TITLED: f32 = 300.0;
pub const ROW: f32 = 20.0;
/// A group's name over its panel, and a page's over its groups while searching.
pub const GROUP_ROW: f32 = 14.0;
pub const PAGE_ROW: f32 = 16.0;
/// Space under a group's panel, and more before the next page's name.
pub const GROUP_GAP: f32 = 10.0;
pub const PAGE_GAP: f32 = 6.0;
/// The control on the right of a row (a button, a box or a slider), and the
/// left column: the search field, the page list and the buttons.
pub const CONTROL_W: f32 = 112.0;
pub const CONTROL_H: f32 = 16.0;
pub const SEARCH_H: f32 = 16.0;
/// Vanilla's scroller: the column `ui::Scroll::press` takes.
pub const SCROLLER_W: f32 = 6.0;
/// The strip under the plate that describes a setting.
pub const DOCK_H: f32 = 50.0;
pub const BUTTON_H: f32 = 20.0;
/// A row's name and control, from its group panel's edges.
pub const INSET: f32 = 10.0;

pub fn fill(batch: &mut HudBatch, atlas: &HudAtlas, [x, y, w, h]: [f32; 4], color: u32) {
    batch.fill(atlas, x, y, w, h, color);
}
/// A 1 px ring inside `rect`: the keyboard or controller focus, in white as vanilla draws it.
pub fn ring(batch: &mut HudBatch, atlas: &HudAtlas, [x, y, w, h]: [f32; 4], color: u32) {
    batch.fill(atlas, x, y, w, 1.0, color);
    batch.fill(atlas, x, y + h - 1.0, w, 1.0, color);
    batch.fill(atlas, x, y + 1.0, 1.0, h - 2.0, color);
    batch.fill(atlas, x + w - 1.0, y + 1.0, 1.0, h - 2.0, color);
}
/// The plate and the strip under it: vanilla's list background, as options lists draw it.
pub fn panel(batch: &mut HudBatch, atlas: &HudAtlas, [x, y, w, h]: [f32; 4], in_world: bool) {
    let sprite = if in_world {
        "gui/inworld_menu_list_background"
    } else {
        "gui/menu_list_background"
    };
    ui::tiled(batch, atlas, sprite, x, y, w, h, 32.0, WHITE);
}
/// The search field and other wells: vanilla's text field, highlighted while focused.
pub fn well(batch: &mut HudBatch, atlas: &HudAtlas, [x, y, w, h]: [f32; 4], focused: bool) {
    let sprite = if focused {
        "widget/text_field_highlighted"
    } else {
        "widget/text_field"
    };
    ui::nine_slice(batch, atlas, sprite, x, y, w, h, WHITE);
}
/// Vanilla's scroller: its background on the track, the scroller on the thumb.
pub fn scroller(batch: &mut HudBatch, atlas: &HudAtlas, track: [f32; 4], thumb: [f32; 4]) {
    let [x, y, w, h] = track;
    ui::nine_slice(
        batch,
        atlas,
        "widget/scroller_background",
        x,
        y,
        w,
        h,
        WHITE,
    );
    let [x, y, w, h] = thumb;
    ui::nine_slice(batch, atlas, "widget/scroller", x, y, w, h, WHITE);
}
/// `text` cut to `width` with an ellipsis, as a long name gives way to its control: the HUD
/// kit's `fit::ellipsize`.
pub use crate::hud_kit::fit::ellipsize as fit;
/// Vanilla's 17 px check box ending at `right`, centred in the row at `y`; `lit` when pointed at or focused.
pub fn checkbox(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    right: f32,
    y: f32,
    on: bool,
    (lit, live): (bool, bool),
) {
    let sprite = match (on, lit && live) {
        (true, true) => "widget/checkbox_selected_highlighted",
        (true, false) => "widget/checkbox_selected",
        (false, true) => "widget/checkbox_highlighted",
        (false, false) => "widget/checkbox",
    };
    let tint = if live { WHITE } else { muted() };
    let top = y + ((ROW - 17.0) / 2.0).floor();
    ui::nine_slice(batch, atlas, sprite, right - 17.0, top, 17.0, 17.0, tint);
}
/// Vanilla's button with its label centred; pointed at or focused, its highlighted sprite.
pub fn button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    label: &Text,
    flags: (bool, bool, bool),
) {
    button_with_room(batch, atlas, rect, label, flags, 0.0);
}
/// A button whose label leaves `room` at its left, for a controller's glyph drawn there
/// (`widgets::action_glyph_room`).
pub fn button_with_room(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    label: &Text,
    (hover, live, _): (bool, bool, bool),
    room: f32,
) {
    let [x, y, w, h] = rect;
    let sprite = match (live, hover) {
        (false, _) => "widget/button_disabled",
        (true, true) => "widget/button_highlighted",
        (true, false) => "widget/button",
    };
    ui::nine_slice(batch, atlas, sprite, x, y, w, h, WHITE);
    let color = if live { WHITE } else { muted() };
    let left = x + 2.0 + room;
    scrolling_label(batch, atlas, label, [left, w - 4.0 - room], (y, h), color);
}
/// A widget label of `[left, width]` inside the widget's `(top, height)`: centred when it fits,
/// else slid (vanilla `AbstractWidget.extractScrollingStringOverContents`) and clipped to the box.
fn scrolling_label(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    [left, width]: [f32; 2],
    (top, height): (f32, f32),
    color: u32,
) {
    ui::scrolling_text(
        batch,
        atlas,
        text,
        left,
        left + width,
        top,
        top + height,
        color,
    );
}
/// What a slider's sprites follow, as `AbstractSliderButton.getSprite` and `getHandleSprite`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SliderState {
    pub hover: bool,
    pub focused: bool,
    /// `canChangeValue`: the keyboard changes its value (Enter pressed on it).
    pub editing: bool,
    /// Dragged with the mouse.
    pub held: bool,
    pub live: bool,
}
/// Vanilla's slider: track and 8 px handle at `position` (0..1) lit as `AbstractSliderButton`'s, the value over them.
pub fn slider(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    value: &Text,
    position: f32,
    state: SliderState,
) {
    let [x, y, w, h] = rect;
    let live = state.live;
    let tint = if live { WHITE } else { muted() };
    let (track, handle) =
        ui::slider_sprites_ext(state.hover, live, state.focused, state.editing, state.held);
    ui::nine_slice(batch, atlas, track, x, y, w, h, tint);
    let at = x + (position.clamp(0.0, 1.0) * (w - 8.0)).floor();
    ui::nine_slice(batch, atlas, handle, at, y, 8.0, h, tint);
    let color = if live { WHITE } else { muted() };
    scrolling_label(batch, atlas, value, [x + 2.0, w - 4.0], (y, h), color);
}
/// A value that a press steps through: a button showing the value, as vanilla's `CycleButton`.
pub fn cycler(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    rect: [f32; 4],
    value: &Text,
    (hover, live): (bool, bool),
) {
    button_with_room(batch, atlas, rect, value, (hover, live, false), 0.0);
}
/// A screen's title, centred at the top.
pub fn title(batch: &mut HudBatch, atlas: &HudAtlas, text: &Text) {
    let w = batch.width;
    ui::centered_text(batch, atlas, text, 0.0, 12.0, w, u32::MAX);
}
/// The cost's colour in the HUD's encoding: the square beside its name.
pub fn cost_color(cost: Cost) -> u32 {
    hex(cost.rgb(), 255)
}

/// An atlas of the vanilla widget sprites and menu backgrounds the settings screens draw, as a pack lays them out.
#[cfg(test)]
pub(crate) fn widget_atlas() -> HudAtlas {
    let root = tempfile::tempdir().unwrap();
    write_widget_resources(root.path());
    HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
        root.path().to_owned(),
    ))
    .unwrap()
}

/// Writes [`widget_atlas`]'s resources under `root`, as the game's own resources lay them out.
#[cfg(test)]
pub(crate) fn write_widget_resources(root: &std::path::Path) {
    let assets = root.join("assets/minecraft");
    std::fs::create_dir_all(assets.join("atlases")).unwrap();
    std::fs::write(
        assets.join("atlases/gui.json"),
        r#"{"sources":[{"type":"minecraft:directory","prefix":"","source":"gui/sprites"}]}"#,
    )
    .unwrap();
    let textures = assets.join("textures/gui");
    for (name, w, h) in [
        ("sprites/widget/checkbox", 17, 17),
        ("sprites/widget/checkbox_highlighted", 17, 17),
        ("sprites/widget/checkbox_selected", 17, 17),
        ("sprites/widget/checkbox_selected_highlighted", 17, 17),
        ("sprites/widget/button", 200, 20),
        ("sprites/widget/button_highlighted", 200, 20),
        ("sprites/widget/button_disabled", 200, 20),
        ("sprites/widget/text_field", 200, 20),
        ("sprites/widget/text_field_highlighted", 200, 20),
        ("sprites/widget/scroller", 6, 32),
        ("sprites/widget/scroller_background", 6, 32),
        ("menu_background", 32, 32),
        ("inworld_menu_background", 32, 32),
        ("menu_list_background", 32, 32),
        ("inworld_menu_list_background", 32, 32),
    ] {
        let path = textures.join(format!("{name}.png"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels = vec![255; (w * h * 4) as usize];
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
    }
}

/// Whether `batch` drew the whole of `r`'s texture, stretched over `at` when given.
#[cfg(test)]
pub(crate) fn drew_rect(batch: &HudBatch, r: &crate::hud::Rect, at: Option<[f32; 4]>) -> bool {
    let corner = |pos: Option<[f32; 2]>, uv: [f32; 2]| {
        batch
            .vertices
            .iter()
            .any(|v| v.uv == uv && pos.is_none_or(|p| v.pos == p))
    };
    let (top_left, bottom_right) = match at {
        Some([x, y, w, h]) => (Some([x, y]), Some([x + w, y + h])),
        None => (None, None),
    };
    corner(top_left, [r.u0, r.v0]) && corner(bottom_right, [r.u1, r.v1])
}

/// Whether `batch` drew the whole of vanilla's GUI sprite `id` somewhere.
#[cfg(test)]
pub(crate) fn drew_sprite(batch: &HudBatch, atlas: &HudAtlas, id: &str) -> bool {
    let r = atlas
        .gui_rect(id)
        .unwrap_or_else(|| panic!("{id} is in the atlas"));
    drew_rect(batch, r, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::take_gui_sprite_misses;

    /// Every painter draws the vanilla sprite its state names, so none comes out blank (#3715).
    #[test]
    fn each_widget_draws_vanilla_s_sprite_for_its_state() {
        let atlas = widget_atlas();
        take_gui_sprite_misses();
        let rect = [10.0, 20.0, 100.0, 20.0];
        let label = Text::empty();
        let draws: [(&str, fn(&mut HudBatch, &HudAtlas)); 9] = [
            ("widget/checkbox", |b, a| {
                checkbox(b, a, 100.0, 0.0, false, (false, true))
            }),
            ("widget/checkbox_highlighted", |b, a| {
                checkbox(b, a, 100.0, 0.0, false, (true, true))
            }),
            ("widget/checkbox_selected", |b, a| {
                checkbox(b, a, 100.0, 0.0, true, (false, true))
            }),
            ("widget/checkbox_selected_highlighted", |b, a| {
                checkbox(b, a, 100.0, 0.0, true, (true, true))
            }),
            ("widget/button", |b, a| {
                button(
                    b,
                    a,
                    [10.0, 20.0, 100.0, 20.0],
                    &Text::empty(),
                    (false, true, false),
                )
            }),
            ("widget/button_highlighted", |b, a| {
                button(
                    b,
                    a,
                    [10.0, 20.0, 100.0, 20.0],
                    &Text::empty(),
                    (true, true, false),
                )
            }),
            ("widget/button_disabled", |b, a| {
                button(
                    b,
                    a,
                    [10.0, 20.0, 100.0, 20.0],
                    &Text::empty(),
                    (false, false, false),
                )
            }),
            ("widget/text_field", |b, a| {
                well(b, a, [10.0, 20.0, 100.0, 20.0], false)
            }),
            ("widget/text_field_highlighted", |b, a| {
                well(b, a, [10.0, 20.0, 100.0, 20.0], true)
            }),
        ];
        for (id, draw) in draws {
            let mut batch = HudBatch::scratch([640, 480]);
            draw(&mut batch, &atlas);
            assert!(drew_sprite(&batch, &atlas, id), "{id} drawn");
        }
        let mut batch = HudBatch::scratch([640, 480]);
        cycler(&mut batch, &atlas, rect, &label, (false, true));
        assert!(
            drew_sprite(&batch, &atlas, "widget/button"),
            "the cycler's button drawn"
        );
        // The track and the thumb each on its own rectangle, so swapped sprites fail.
        let (track, thumb) = ([0.0, 0.0, 6.0, 200.0], [0.0, 40.0, 6.0, 40.0]);
        let mut batch = HudBatch::scratch([640, 480]);
        scroller(&mut batch, &atlas, track, thumb);
        let at = |id: &str, rect| drew_rect(&batch, atlas.gui_rect(id).unwrap(), Some(rect));
        assert!(
            at("widget/scroller_background", track),
            "the track's sprite on the track"
        );
        assert!(
            at("widget/scroller", thumb),
            "the scroller's sprite on the thumb"
        );
        assert_eq!(take_gui_sprite_misses(), Vec::<String>::new());
    }

    /// A wrong sprite name, as #2522's file paths were, draws the missing texture and is reported.
    #[test]
    fn a_wrong_sprite_name_draws_the_missing_texture_and_is_reported() {
        let atlas = widget_atlas();
        take_gui_sprite_misses();
        let mut batch = HudBatch::scratch([640, 480]);
        let id = "gui/sprites/widget/checkbox";
        ui::nine_slice(&mut batch, &atlas, id, 0.0, 0.0, 17.0, 17.0, WHITE);
        assert_eq!(take_gui_sprite_misses(), [id]);
        let missing = &atlas.sprites[crate::hud::MISSING_SPRITE];
        assert!(drew_rect(&batch, missing, Some([0.0, 0.0, 17.0, 17.0])));
    }

    /// The settings screens name only sprites the game's own 26.3 resources have.
    #[test]
    #[ignore = "daily"]
    fn the_settings_screens_name_only_sprites_the_game_has() {
        use crate::settings_shell::{Hosted, Shell};
        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client/26.3")
            });
        let atlas = HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
            root,
        ))
        .unwrap();
        let s = crate::settings::Settings::default();
        take_gui_sprite_misses();
        fn frame(
            shell: &mut dyn Hosted,
            s: &crate::settings::Settings,
            atlas: &HudAtlas,
            in_world: bool,
        ) {
            let mut batch = HudBatch::scratch([1708, 960]);
            shell.draw_test_frame(
                s,
                &mut batch,
                atlas,
                (0.0, 0.0),
                in_world,
                None,
                &mut |_, _| {},
            );
            crate::widgets::end_frame(1);
        }
        for in_world in [false, true] {
            frame(
                &mut Shell::<crate::extras_ui::ExtrasScreen>::default(),
                &s,
                &atlas,
                in_world,
            );
            frame(
                &mut Shell::<crate::graphics_ui::GraphicsScreen>::default(),
                &s,
                &atlas,
                in_world,
            );
            frame(
                &mut Shell::<crate::controller_ui::ControllerScreen>::default(),
                &s,
                &atlas,
                in_world,
            );
        }
        assert_eq!(take_gui_sprite_misses(), Vec::<String>::new());
    }

    fn atlas() -> HudAtlas {
        HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
            (std::path::Path::new("/nonexistent/sunrise-test-assets")).to_owned(),
        ))
        .unwrap()
    }

    fn x_span(batch: &HudBatch, plain: &HudBatch) -> (f32, f32) {
        let label = |v: &&crate::hud::HudVertex| !plain.vertices.iter().any(|p| p.pos == v.pos);
        batch
            .vertices
            .iter()
            .filter(label)
            .map(|v| v.pos[0])
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            })
    }

    /// A label wider than its widget slides inside the widget's box instead of being cut by
    /// characters; a short one stays centred.
    #[test]
    fn long_labels_stay_in_the_widget_and_short_ones_centred() {
        let atlas = atlas();
        let rect = [10.0, 20.0, 100.0, 20.0];
        let long = Text::literal("Graphics API: Prefer Vulkan (Experimental)");
        assert!(atlas.font.width_text(&long.resolved()) > rect[2]);
        let short = Text::literal("Done");
        let short_w = atlas.font.width_text(&short.resolved());
        let draws: [(&str, fn(&mut HudBatch, &HudAtlas, [f32; 4], &Text)); 3] = [
            ("button", |b, a, r, t| {
                button(b, a, r, t, (false, true, false))
            }),
            ("slider", |b, a, r, t| {
                slider(
                    b,
                    a,
                    r,
                    t,
                    0.5,
                    SliderState {
                        live: true,
                        ..Default::default()
                    },
                )
            }),
            ("cycler", |b, a, r, t| cycler(b, a, r, t, (false, true))),
        ];
        for (name, draw) in draws {
            let mut plain = HudBatch::scratch([640, 480]);
            draw(&mut plain, &atlas, rect, &Text::empty());
            let base = plain.vertices.len();
            let mut batch = HudBatch::scratch([640, 480]);
            draw(&mut batch, &atlas, rect, &long);
            assert!(batch.vertices.len() > base, "{name}: long label drawn");
            // The widget's own fill, track and handle also draw: only the label's quads count.
            let widget: Vec<_> = plain.quad_scissors().map(|(bounds, _)| bounds).collect();
            let label = batch
                .quad_scissors()
                .zip(batch.visible_quads())
                .filter(|((bounds, _), _)| !widget.contains(bounds))
                .filter_map(|(_, visible)| visible);
            for [x0, y0, x1, y1] in label {
                assert!(
                    rect[0] <= x0
                        && x1 <= rect[0] + rect[2]
                        && rect[1] <= y0
                        && y1 <= rect[1] + rect[3],
                    "{name}: {:?} outside the widget",
                    [x0, y0, x1, y1]
                );
            }
            // The label's own glyphs (past the widget's fill) start centred.
            let mut batch = HudBatch::scratch([640, 480]);
            draw(&mut batch, &atlas, rect, &short);
            let (lo, hi) = x_span(&batch, &plain);
            let middle = (lo + hi) / 2.0;
            let want = rect[0] + rect[2] / 2.0;
            assert!(
                (middle - want).abs() <= short_w / 2.0 + 2.0 && (middle - want).abs() < 3.0,
                "{name}: centre {middle}, want {want}"
            );
        }
    }
}
