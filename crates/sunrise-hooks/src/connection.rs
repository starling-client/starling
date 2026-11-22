//! The window's connection to a server (docs/layers.md, "How a Sunrise crate plugs in"): what
//! [`crate::Hooks`] hears as one starts, lets the player in and closes, and how a crate has the game
//! connect again after one was lost. The engine shows the Disconnected screen, its line and its
//! Cancel button, and makes the connection; the crate decides whether, when and what the line says.
//!
//! Each connection over the network is one session: [`crate::Hooks::connecting`], at most one
//! [`crate::Hooks::joined`], then exactly one of [`crate::Hooks::left`] or [`crate::Hooks::ended`],
//! however it closes and whatever screen follows. A world the window plays itself (single player,
//! its own LAN world) is no such session and calls none of them.

/// A server the window connects to over the network.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Server {
    /// As the player gave it: `host`, `host:port`, an invite code.
    pub address: String,
    /// The player name it connects with.
    pub name: String,
    /// Found on the local network (the server list's LAN worlds).
    pub lan: bool,
}

impl Server {
    pub fn new(address: impl Into<String>, name: impl Into<String>, lan: bool) -> Self {
        Self {
            address: address.into(),
            name: name.into(),
            lan,
        }
    }
}

/// How a connection closed when it wasn't lost ([`crate::Hooks::left`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Left {
    /// The player disconnected (the pause screen's Disconnect, a command or tool that leaves).
    Disconnected,
    /// The game closed while connected: its window, or the quit it was asked for.
    Quit,
    /// The server sent the window on to another server (`ClientboundTransferPacket`):
    /// [`crate::Hooks::connecting`] follows at once.
    Transferred,
}

/// What ended a connection the player didn't leave ([`Ended::by`]). The category
/// alone does not say whether to retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum By {
    /// The connection itself: it couldn't be made, timed out or was lost.
    Connection,
    /// The server, or a refusal on the way in: a kick, a ban, a full or stopping server.
    Server,
    /// This client: a step the join needs failed or was declined (a required resource pack, the
    /// code of conduct), or a mod disconnected.
    Client,
}

/// How a connection ended when the player didn't leave ([`crate::Hooks::ended`]).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Ended<'a> {
    /// The reason, as the Disconnected screen shows it, in the player's language.
    pub reason: &'a str,
    /// Its translation key when it had one (`multiplayer.disconnect.kicked`), whatever language
    /// the text is in.
    pub key: Option<&'a str>,
    /// What ended it.
    pub by: By,
}

impl<'a> Ended<'a> {
    pub fn new(reason: &'a str, key: Option<&'a str>, by: By) -> Self {
        Self { reason, key, by }
    }
}

/// What a crate answers while the Disconnected screen offers to connect again
/// ([`crate::Hooks::retry`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Retry {
    /// Not yet: the line the screen shows under the reason, above its Cancel button.
    Wait(String),
    /// Connect to this server now.
    Now(Server),
}
