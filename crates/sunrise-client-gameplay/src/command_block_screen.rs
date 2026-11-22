//! The command block edit screen's state and layout (`AbstractCommandBlockEditScreen`).
use crate::screen_info::{Tr, tr};
use sunrise_client::{
    chat::{ChatScreen, Commands},
    command_block::{CommandBlock, Mode},
    screen_draw::Measure,
    text::Text,
    ui::EditBox,
};

pub type Pos = (i32, i32, i32);

/// The open `CommandBlockEditScreen`.
pub struct CommandBlockScreen {
    pub pos: Pos,
    /// The command block minecart being edited (`MinecartCommandBlockEditScreen`);
    /// `pos` is unused then.
    pub cart: Option<u32>,
    /// The command box with its `CommandSuggestions`.
    pub command: ChatScreen,
    /// When the command box was focused, for its cursor's blink.
    pub opened: std::time::Instant,
    pub previous: EditBox,
    pub track_output: bool,
    pub mode: Mode,
    pub conditional: bool,
    pub auto: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Output,
    Mode,
    Conditional,
    Auto,
    Done,
    Cancel,
}

/// `AbstractCommandBlockEditScreen.getPreviousY`.
pub fn previous_y(cart: bool) -> f32 {
    if cart { 150.0 } else { 135.0 }
}

/// `EditBox.textColorUneditable`.
const UNEDITABLE: u32 = 0xFF70_7070;

impl CommandBlockScreen {
    /// `AbstractCommandBlockEditScreen.init`: the command box and the previous output box.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        m: &Measure,
        c: &Commands,
        (w, h): (f32, f32),
        (pos, cart): (Pos, Option<u32>),
        block: &CommandBlock,
        (mode, conditional): (Mode, bool),
        auto_suggestions: bool,
        opened: std::time::Instant,
    ) -> Self {
        let mut command = ChatScreen::command_block(m, c, &block.command, w as i32, h as i32);
        command.auto_suggestions = auto_suggestions;
        command.input.message = tr("advMode.command", &[]);
        let x = (w / 2.0).floor() - 150.0;
        let mut previous = EditBox::new(x, previous_y(cart.is_some()), 276.0, 20.0);
        previous.message = tr("advMode.previousOutput", &[]);
        previous.max_length = 32500;
        previous.text_color = UNEDITABLE;
        previous.focused = false;
        let mut screen = Self {
            pos,
            cart,
            command,
            opened,
            previous,
            track_output: block.track_output,
            mode,
            conditional,
            auto: block.auto,
        };
        screen.show_previous(block);
        screen
    }
    /// `AbstractCommandBlockEditScreen.init` and `CommandBlockEditScreen`'s
    /// buttons, in GUI pixels of a `w`×`h` screen.
    /// A minecart's screen has no mode buttons (`getPreviousY` 150).
    /// Laid out once per build (`CommandBlockLayout`), not per frame.
    pub fn buttons(w: f32, h: f32, cart: bool) -> Vec<(Button, f32, f32, f32)> {
        let mid = (w / 2.0).floor();
        let low = (h / 4.0).floor() + 132.0;
        let mut out = vec![(Button::Output, mid + 130.0, previous_y(cart), 20.0)];
        if !cart {
            out.extend([
                (Button::Mode, mid - 154.0, 165.0, 100.0),
                (Button::Conditional, mid - 50.0, 165.0, 100.0),
                (Button::Auto, mid + 54.0, 165.0, 100.0),
            ]);
        }
        out.extend([
            (Button::Done, mid - 154.0, low, 150.0),
            (Button::Cancel, mid + 4.0, low, 150.0),
        ]);
        out
    }
    /// What button `b` narrates: the switches are `CycleButton`s that show
    /// only their value (`displayOnlyValue`), "<name>: <value> button" and
    /// the value a click moves to.
    pub fn narration(
        &self,
        b: Button,
        rect: [f32; 4],
        tr: Tr<'_>,
    ) -> sunrise_client::widgets::Narratable {
        let state = (self.track_output, self.mode, self.conditional, self.auto);
        button_narration(b, state, rect, tr)
    }
    pub fn label(&self, b: Button) -> Text {
        let tr = |key: &str| Text::translate(key, &[]);
        match b {
            Button::Output => Text::literal(if self.track_output { "O" } else { "X" }),
            Button::Mode => tr(self.mode.label()),
            Button::Conditional => tr(if self.conditional {
                "advMode.mode.conditional"
            } else {
                "advMode.mode.unconditional"
            }),
            Button::Auto => tr(if self.auto {
                "advMode.mode.autoexec.bat"
            } else {
                "advMode.mode.redstoneTriggered"
            }),
            Button::Done => tr("gui.done"),
            Button::Cancel => tr("gui.cancel"),
        }
    }
    pub fn pos(&self) -> Pos {
        self.pos
    }
    /// A minecart's edit screen, not a command block's.
    pub fn cart(&self) -> bool {
        self.cart.is_some()
    }
    /// The previous output box's rectangle.
    pub fn previous(&self) -> [f32; 4] {
        let p = &self.previous;
        [p.x, p.y, p.width, p.height]
    }
    /// `AbstractCommandBlockEditScreen.resize`: the command box is made
    /// again at the new size with its text (`ChatScreen::resize`), and the
    /// previous output box is centred on it. The screen's build calls it,
    /// never the draw.
    pub fn resize(&mut self, m: &Measure, c: &Commands, w: f32, h: f32) {
        self.command.resize(m, c, w as i32, h as i32);
        self.previous.x = (w / 2.0).floor() - 150.0;
    }
    /// `updatePreviousOutput`.
    pub fn show_previous(&mut self, block: &CommandBlock) {
        let shown = if self.track_output {
            block.last_output_string()
        } else {
            "-".to_owned()
        };
        self.previous.set_value(&shown);
    }
}

/// [`CommandBlockScreen::narration`] of the switches' state (track output,
/// mode, conditional, automatic).
fn button_narration(
    b: Button,
    (track_output, mode, conditional_on, auto_on): (bool, Mode, bool, bool),
    rect: [f32; 4],
    tr: Tr<'_>,
) -> sunrise_client::widgets::Narratable {
    use sunrise_client::widgets::Narratable;
    let full_tr = tr;
    let tr = |key: &str| full_tr(key, &[]);
    let o_x = |on: bool| if on { "O" } else { "X" }.to_owned();
    let conditional = |on: bool| {
        tr(if on {
            "advMode.mode.conditional"
        } else {
            "advMode.mode.unconditional"
        })
    };
    let auto = |on: bool| {
        tr(if on {
            "advMode.mode.autoexec.bat"
        } else {
            "advMode.mode.redstoneTriggered"
        })
    };
    let (name, value, next) = match b {
        Button::Output => ("advMode.trackOutput", o_x(track_output), o_x(!track_output)),
        Button::Mode => ("advMode.mode", tr(mode.label()), tr(mode.next().label())),
        Button::Conditional => (
            "advMode.type",
            conditional(conditional_on),
            conditional(!conditional_on),
        ),
        Button::Auto => ("advMode.triggering", auto(auto_on), auto(!auto_on)),
        Button::Done => return Narratable::button(tr("gui.done"), rect, true),
        Button::Cancel => return Narratable::button(tr("gui.cancel"), rect, true),
    };
    let full = full_tr("options.generic_value", &[&tr(name), &value]);
    Narratable::cycle(full, next, rect, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minecart_screen_has_no_mode_buttons_and_a_lower_output_row() {
        let cart = CommandBlockScreen::buttons(400.0, 240.0, true);
        let kinds: Vec<_> = cart.iter().map(|b| b.0).collect();
        assert!(kinds == [Button::Output, Button::Done, Button::Cancel]);
        // MinecartCommandBlockEditScreen.getPreviousY.
        assert_eq!(cart[0].2, 150.0);
        let block = CommandBlockScreen::buttons(400.0, 240.0, false);
        assert_eq!(block.len(), 6);
        assert_eq!(block[0].2, 135.0);
        // Done and Cancel sit at the same height on both.
        assert_eq!(cart[1].2, block[4].2);
    }
}

#[cfg(test)]
mod narration_tests {
    use super::*;
    use sunrise_client::widgets::What;

    fn keys(key: &str, args: &[&str]) -> String {
        if args.is_empty() {
            key.to_owned()
        } else {
            format!("{key}({})", args.join(", "))
        }
    }

    #[test]
    fn the_switches_narrate_as_cycle_buttons_showing_their_value() {
        let state = (true, Mode::Redstone, false, true);
        let cycle = |b| button_narration(b, state, [0.0; 4], &keys).what;
        let expect = |message: &str, next: &str| What::Cycle {
            message: message.into(),
            next: next.into(),
            active: true,
        };
        assert_eq!(
            cycle(Button::Output),
            expect("options.generic_value(advMode.trackOutput, O)", "X")
        );
        assert_eq!(
            cycle(Button::Mode),
            expect(
                "options.generic_value(advMode.mode, advMode.mode.redstone)",
                "advMode.mode.sequence"
            )
        );
        assert_eq!(
            cycle(Button::Conditional),
            expect(
                "options.generic_value(advMode.type, advMode.mode.unconditional)",
                "advMode.mode.conditional"
            )
        );
        assert_eq!(
            cycle(Button::Auto),
            expect(
                "options.generic_value(advMode.triggering, advMode.mode.autoexec.bat)",
                "advMode.mode.redstoneTriggered"
            )
        );
        assert_eq!(
            cycle(Button::Done),
            What::Button {
                message: "gui.done".into(),
                active: true
            }
        );
    }
}
