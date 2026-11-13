//! The `input_key` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0125

pub trait Hooks {
    /// A real window key event after handling, including repeats and consumed shortcuts
    /// (`InputEvent.Key`). `key` is vanilla's name for the physical key (`key.keyboard.a`,
    /// `key.keyboard.unknown` for one it doesn't name), `action` what it did, and the
    /// modifier bits the screen hooks' GLFW encoding. Not cancellable.
    #[hook(side = "client", contract = "listen", neoforge = "InputEvent.Key")]
    fn input_key();
}
