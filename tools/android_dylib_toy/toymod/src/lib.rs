//! The toy mod: its entry bumps the game's counter and hands back hooks, with a heap value the game frees.
struct Mod;

impl toygame::Hooks for Mod {
    fn hello(&self) -> String {
        format!("toymod sees count {}", toygame::bump())
    }
}

/// The mod's entry, a Rust function as `horizon_native_mod_v1` is.
#[unsafe(no_mangle)]
pub fn toy_mod_v1() -> toygame::Entry {
    toygame::bump();
    toygame::Entry {
        id: "toymod",
        hooks: Box::new(Mod),
    }
}
