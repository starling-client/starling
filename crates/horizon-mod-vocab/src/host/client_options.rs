//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_options` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ClientOptionsClientHost {
    /// Side: client.
    /// A snapshot of the current input preferences.
    #[access(ClientSetup)]
    fn input_options(&self) -> ClientInputOptions;
    /// Side: client.
    /// A snapshot of the current graphics preferences.
    #[access(ClientSetup)]
    fn graphics_options(&self) -> ClientGraphicsOptions;
    /// Side: client.
    /// A snapshot of the current GUI and accessibility preferences.
    #[access(ClientSetup)]
    fn gui_options(&self) -> ClientGuiOptions;
    /// Side: client.
    /// A snapshot of the current chat preferences.
    #[access(ClientSetup)]
    fn chat_options(&self) -> ClientChatOptions;
    /// Side: client.
    /// A snapshot of the current audio preferences.
    #[access(ClientSetup)]
    fn audio_options(&self) -> ClientAudioOptions;
}
