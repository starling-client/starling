//! The `WorldDrawContext::name_tag` scene (#4171). It submits in `submit_custom_geometry`
//! only in a view whose game time is `SCENE_TIME`, so other tests' views stay empty.
use mod_api::{
    DrawError, NameTagOptions, SubmitCustomGeometryEvent, Text, WorldDepth, WorldLine,
    text::TextStyle,
};

/// The game time of the one view this scene draws in.
pub const SCENE_TIME: i64 = 4171;

/// The scene's labels: one of each kind the offscreen tests pin, then one refused.
pub fn scene() -> Vec<([f64; 3], Text, NameTagOptions)> {
    let mut bold = TextStyle::default();
    bold.bold = Some(true);
    vec![
        // Hidden behind a wall: one translucent copy, in light 0 as given.
        (
            [0.5, 70.0, 0.5],
            Text::literal("Hidden"),
            NameTagOptions::new(false, true, 0x00),
        ),
        // See-through in light 0: the copy in the world at light 2, the faded one unlit.
        (
            [-1.0, 70.3, 1.5],
            Text::literal("Through"),
            NameTagOptions::new(true, true, 0x00),
        ),
        // Styles counted in the centring width; no backdrop; full sky light.
        (
            [1.5, 69.6, 2.5],
            Text::literal("Bold").with_style(bold),
            NameTagOptions::new(false, false, 0xF0),
        ),
        // Refused by name; draws nothing.
        (
            [f64::NAN, 70.0, 0.5],
            Text::literal("NaN"),
            NameTagOptions::new(true, true, 0xF0),
        ),
    ]
}

pub fn submit_custom_geometry(event: &mut SubmitCustomGeometryEvent<'_>) {
    if event.view.game_time != SCENE_TIME {
        return;
    }
    let camera = event.view.camera_position();
    for (n, (at, text, options)) in scene().into_iter().enumerate() {
        let code = result_code(event.draw.name_tag(at, &text, options));
        // Each call's result as a marker line one
        // block ahead, `code` blocks to the right, `n` blocks up (see `RESULT_CODES`).
        let from = [
            camera[0] + f64::from(code),
            camera[1] + n as f64,
            camera[2] - 1.0,
        ];
        let to = [from[0] + 0.5, from[1], from[2]];
        let _ = event
            .draw
            .lines(&[WorldLine::new(from, to, !0, 1.0, WorldDepth::Tested)]);
    }
}

/// The results [`scene`]'s calls must return, coded by [`result_code`]: three drawn, NaN refused.
pub const RESULT_CODES: [u32; 4] = [0, 0, 0, 1];

/// A `name_tag` result as a number: 0 drawn, then each refusal.
pub fn result_code(result: Result<(), DrawError>) -> u32 {
    match result {
        Ok(()) => 0,
        Err(DrawError::NonFinite) => 1,
        Err(DrawError::InvalidWidth) => 2,
        Err(_) => 4,
    }
}
