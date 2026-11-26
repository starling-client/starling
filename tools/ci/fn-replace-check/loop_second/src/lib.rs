//! A second mod beside `fnr_loop`: its block `fnr_loop_second:counter` has a ticking block entity of
//! its own type, and its `[[native.loop]]` slot (`src/loops/own.rs`) counts only those.
#[derive(Default)]
struct Mod;

/// The counter's block and its block entity type.
const COUNTER: &str = "fnr_loop_second:counter";

#[mod_api::sunrise_mod]
impl mod_api::Hooks for Mod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("counter").blocks(&[COUNTER]).build();
        r.block_entity_part(&kind, Counter).unwrap();
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new(COUNTER, "minecraft:stone"))
            .unwrap();
    }
}

/// Makes the counter's block entity tick.
struct Counter;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for Counter {}
#[mod_api::sunrise_part]
impl mod_api::BlockEntityPart for Counter {
    // The loop slots do the observable work; subscribing here binds the resident ticker.
    fn block_entity_tick(&self, _: &mut mod_api::BlockEntityTickEvent<'_>) {}
}

/// Called by the helper: a helper may use its mod's crate.
pub fn note(what: &str) {
    eprintln!("fnr: loop_second {what}");
}
