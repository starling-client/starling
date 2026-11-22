/// The local player's sampled movement input, lent to `movement_input_update` listeners.
///
/// The move vector and the key presses are independent. Editing either leaves the other alone.
// no wire: the client lends this tick's input in-process; only the key presses go to the server.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: the engine samples the independent vector and key fields together by literal.
pub struct MovementInput {
    /// `[left, forward]` movement before item-use and crouching multipliers, applied directly
    /// as in vanilla `ClientInput` and NeoForge `MovementInputUpdateEvent`.
    pub move_vector: [f32; 2],
    /// The input flags; auto-jump can add a jump press after listeners run.
    pub key_presses: MovementKeys,
}

/// The local player's seven independent key presses, including opposing directions.
// no wire: these are hook values, not the protocol's player input packet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: the seven vanilla input flags are sampled and forwarded as one literal.
pub struct MovementKeys {
    /// Forward is pressed.
    pub forward: bool,
    /// Backward is pressed.
    pub backward: bool,
    /// Left is pressed.
    pub left: bool,
    /// Right is pressed.
    pub right: bool,
    /// Jump is pressed, before the game's auto-jump adds its press.
    pub jump: bool,
    /// Sneak is pressed.
    pub sneak: bool,
    /// The sprint key is pressed. This is not the player's sprinting state.
    pub sprint: bool,
}
