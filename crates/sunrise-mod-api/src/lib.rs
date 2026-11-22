//! The Sunrise mod API: the one crate a Sunrise code mod depends on.
//!
//! Sunrise is Minecraft: Java Edition 26.3 written in Rust. A code mod is a Rust crate that implements
//! [`Hooks`] and builds with the Sunrise mod SDK. Add this API to your mod's `Cargo.toml` as
//! `sunrise-mod-api` (the library name is `mod_api`).
//!
//! The [modding book](https://playsunrise.net/docs/modding) is the guide: start with
//! [Your first native mod](https://playsunrise.net/docs/modding/start/first-mod). This reference documents
//! every item of the API.
//!
//! # A first mod
//!
//! ```ignore
//! use mod_api::{self, Hooks, PlayerLoggedInEvent, ServerHost, Text};
//!
//! #[derive(Default)]
//! struct Mod;
//!
//! #[mod_api::sunrise_mod]
//! impl Hooks for Mod {
//!     /// A player joined the server: greet them.
//!     fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
//!         let line = format!("greeter says hello, {}!", event.name);
//!         if let Some(player) = host.player(event.player) {
//!             let _ = player.tell(host, Text::literal(line));
//!         }
//!     }
//! }
//! ```
//!
//! `#[sunrise_mod]` marks the one `impl Hooks` at the crate root and subscribes the mod to exactly the hooks it
//! implements. The book's examples crate compiles this mod against the API.
//!
//! # Where to start
//!
//! | Item | What it is |
//! | --- | --- |
//! | [`Hooks`] | Every moment a mod can answer, one method each, with its side, its NeoForge 26.3 counterpart and whether it can be cancelled. |
//! | [`BlockPart`], [`ItemPart`], [`BlockEntityPart`], [`MenuPart`], [`EntityPart`] | Behaviour for the objects a mod attaches a Part to. |
//! | [`Registrar`] | What a mod declares in [`Hooks::register`]: registry keys, Parts and settings. |
//! | [`Host`], [`ServerHost`], [`ClientHost`] | The game as a hook sees it, split by side so a call on the wrong side does not compile. |
//! | [`AttachmentType`] | Data a mod keeps on levels, chunks, entities and other holders. |
//! | [`Wire`](derive@Wire) | The derive that encodes typed payloads and other values. |
//!
//!
//! # Guides in the book
//!
//! - [Hooks and the Host](https://playsunrise.net/docs/modding/concepts/hooks-and-the-host)
//! - [Load phases and access](https://playsunrise.net/docs/modding/concepts/load-phases)
//! - [Parts](https://playsunrise.net/docs/modding/concepts/parts),
//!   [capabilities](https://playsunrise.net/docs/modding/concepts/capabilities) and
//!   [attachments](https://playsunrise.net/docs/modding/concepts/attachments)
//! - [Wire types](https://playsunrise.net/docs/modding/concepts/wire-types)
//! - [The manifest and the package](https://playsunrise.net/docs/modding/reference/manifest)
extern crate self as mod_api;

pub use mod_vocab::*;
/// Platform-owned crash attribution with shared vocabulary types.
#[doc(hidden)]
pub mod content {
    #[doc(hidden)]
    pub mod crash;
}
/// Platform runtime operations and engine-owned vocabulary installations.
#[doc(hidden)]
pub mod __private;

mod load_state;

mod fluid_state;
mod parts;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
