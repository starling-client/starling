//! The diamond's tip: a mod using `itoa` and depending on two mods that use it too. Each mod is a
//! dylib, so without the launcher's shared registry library rustc refuses to link it ("cannot
//! satisfy dependencies so `itoa` only shows up once").
use mod_api::{Hooks, ServerHost};

#[derive(Default)]
struct Mod;

#[mod_api::sunrise_mod]
impl Hooks for Mod {
    fn server_started(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartedEvent) {
        eprintln!(
            "fnr: shared_tip left {} right {} own {}",
            fnr_shared_left::format(1),
            fnr_shared_right::format(2),
            itoa::Buffer::new().format(3)
        );
    }
}
