use mod_api::Hooks;

#[derive(Default)]
pub struct Mod;

#[mod_api::sunrise_mod]
impl Hooks for Mod {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = registrar.block_entity_type("observer").blocks(&[]).build();
        registrar.block_entity_part(&kind, Dependent).unwrap();
    }
}

struct Dependent;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for Dependent {}
#[mod_api::sunrise_part]
impl mod_api::BlockEntityPart for Dependent {
    fn block_entity_tick(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        event.entity.data = vec![
            unsafe { wasm_test_library::SHARED },
            (&raw mut wasm_test_library::SHARED) as usize as i32,
        ];
    }
}
