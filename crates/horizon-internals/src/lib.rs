//! A native mod's declared game internals.
//!
//! The native build replaces this crate with a facade generated from the mod's `[native]
//! `internals` and `raw` injections. Only declared types and functions are re-exported there;
//! raw listeners have safe constructors that check their function types at compile time.
#![forbid(unsafe_code)]
