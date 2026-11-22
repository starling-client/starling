//! A mod's own source as the mod tools read it, without the generator that builds the game's
//! API: the source files its package carries ([`package`]), the mod folder the uses scan reads
//! ([`mod_uses`]), a crate's module files ([`modules`]).
pub mod index;
pub mod layouts;
pub mod mod_uses;
pub mod modules;
pub mod package;
