//! What a GameEvent's parameter does on the client, for the game-event parameter parity harness: for each
//! float (hex bits, one per line, in the file given), the game mode CHANGE_GAME_MODE sets and whether
//! GUARDIAN_ELDER_EFFECT plays the curse sound.
use sunrise_client::command::args::GameMode;
use sunrise_protocol::packets::play::GameEvent;

pub fn main() {
    let path = super::args()
        .nth(1)
        .expect("usage: game_event_param_fixture INPUTS");
    let inputs = std::fs::read_to_string(path).expect("inputs");
    for line in inputs.lines() {
        let bits = u32::from_str_radix(line.trim(), 16).expect("hex float bits");
        let param = f32::from_bits(bits);
        let mode = GameEvent { event: 3, param }.rounded_param();
        let mode = GameMode::from_id(mode).unwrap_or(GameMode::Survival).id();
        let curse = GameEvent { event: 10, param }.rounded_param() == 1;
        println!("{bits:08x} {mode} {}", u8::from(curse));
    }
}
