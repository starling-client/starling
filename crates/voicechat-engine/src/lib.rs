//! Complex Voice Chat for Horizon: voice chat compatible with servers running Simple Voice Chat
//! 2.6.x (protocol compatibility version 20). Written from the protocol's
//! packet layouts; no code from the mod. Opus is the bundled libopus 1.5.2.
mod client;
mod device;
pub mod icons;
pub mod mic_filter;
mod mixer;
mod opus;
pub mod proto;

pub use client::{
    GroupInfo, Payload, Player, Settings, State, Status, create_group, join, join_group, leave,
    leave_group, maintain, microphones, payload, poll_handshake_retry, push_to_talk,
    set_player_volume, set_settings, set_testing, speakers, status, toggle_disabled, toggle_mute,
    update_listener,
};
pub use device::{is_valid_input_device, is_valid_output_device, set_platform_hooks};
pub use mic_filter::{push_game_audio_reference, set_hardware_volume_scale};
pub use proto::GroupKind;
