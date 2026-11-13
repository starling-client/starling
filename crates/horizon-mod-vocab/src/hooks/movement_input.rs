//! The `movement_input` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Local player movement input, after reading it and before applying it.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "MovementInputUpdateEvent",
        cancellable = false
    )]
    fn movement_input();
}
