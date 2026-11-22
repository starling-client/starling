//! The toy game: a thread-local counter and the trait a mod implements, as `mod_api::Hooks` is.
use std::cell::Cell;

thread_local! {
    static COUNT: Cell<u32> = const { Cell::new(0) };
}

/// One more on this thread's counter: the same counter for the game and every mod only when they share
/// one std and one copy of this crate.
pub fn bump() -> u32 {
    COUNT.with(|count| {
        count.set(count.get() + 1);
        count.get()
    })
}

/// What a mod implements.
pub trait Hooks {
    fn hello(&self) -> String;
}

/// What a mod library hands the game, as `horizon_native_mod_v1` does.
pub struct Entry {
    pub id: &'static str,
    pub hooks: Box<dyn Hooks>,
}
