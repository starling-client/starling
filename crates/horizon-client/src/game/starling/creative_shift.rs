//! Where the creative window is from the middle of the screen (`creative_origin`).
//!
//! In a module of its own: horizon-mixins reads a macro among `game`'s items as something that
//! could shadow `format!`, `matches!` and `vec!`, and then refuses to hook any function under
//! `game` that uses them (E_UNSUPPORTED_MACRO).
thread_local! {
    pub(super) static CREATIVE_SHIFT: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}
