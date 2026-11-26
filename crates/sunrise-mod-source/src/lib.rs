//! A mod's own source as the mod tools read it, without the generator that builds the game's
//! API: the source files its package carries ([`package`]), the mod folder the uses scan reads
//! ([`mod_uses`]), a crate's module files ([`modules`]).
#[cfg(feature = "scan")]
pub mod index;
#[cfg(feature = "scan")]
pub mod layouts;
#[cfg(feature = "scan")]
pub mod mod_uses;
#[cfg(feature = "scan")]
pub mod modules;
pub mod package;
