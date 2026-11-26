//! The Rust the modding book includes with `{{#include}}`. A snippet lives here, between
//! `ANCHOR` comments, so a change to the mod API that breaks it fails the build instead of
//! leaving a stale page.

use mod_api::{
    AttachmentDescriptor, AttachmentError, AttachmentKey, Holder, Hooks, Registrar,
    RegistrationContext, ServerHost, ServerTickEvent,
};
use std::sync::RwLock;

// ANCHOR: counter
/// A counter on the level, registered once and held by its issued key.
#[derive(Default)]
struct Mod {
    counter: RwLock<Option<AttachmentKey<i32>>>,
}
// ANCHOR_END: counter

// ANCHOR: tick_hook
#[mod_api::sunrise_mod]
impl Hooks for Mod {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        if let Ok(key) =
            registrar.register_attachment_type(AttachmentDescriptor::new("example:counter", || 0))
        {
            *self.counter.write().unwrap() = Some(key);
        }
    }

    fn server_tick(&self, host: &mut dyn ServerHost, _: &mut ServerTickEvent<'_>) {
        if let Some(counter) = self.counter.read().unwrap().clone() {
            let _ = count(host, &counter);
        }
    }
}
// ANCHOR_END: tick_hook

// ANCHOR: count
fn count(host: &mut dyn ServerHost, counter: &AttachmentKey<i32>) -> Result<(), AttachmentError> {
    let old = counter.get(host, Holder::Level, |n| *n)?;
    counter.set(host, Holder::Level, old + 1)
}
// ANCHOR_END: count

// ANCHOR: menu_quick_move
/// Shift-click a slot in the local player's current menu.
pub fn quick_move(host: &mut dyn mod_api::ClientHost, slot: usize) -> bool {
    let Some(menu) = host
        .local_player()
        .and_then(|player| player.container_menu(host))
    else {
        return false;
    };
    menu.click(host, &[mod_api::MenuClick::QuickMove { slot }])
}
// ANCHOR_END: menu_quick_move

// Every standalone snippet module under src is compiled.
include!(concat!(env!("OUT_DIR"), "/example_modules.rs"));
