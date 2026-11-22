use super::*;
use mod_api::RandomSource as _;
thread_local! {
    static DRAWS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}
struct Sparks;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for Sparks {
    fn animate_tick(
        &self,
        _: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::AnimateTickEvent<'_>,
    ) {
        assert_eq!(event.pos, mod_api::BlockPos::new(5, 63, 4));
        let draws = format!(
            "{} {} {} {}",
            event.random.next_float(),
            event.random.next_int_bound(10),
            event.random.next_boolean(),
            event.random.next_double()
        );
        DRAWS.with_borrow_mut(|all| all.push(draws));
    }
}
/// A guest part's draws reach the game's random, in order, and move it.
#[test]
fn wire_loopback_animate_tick_draws_the_game_random() {
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    install_counter();
    // The platform phase this scope sets is process-wide state another test may not have installed yet.
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let index = guest::add_handler(vec![guest::view::<dyn mod_api::BlockPart>(Arc::new(
        Sparks,
    ))]);
    let proxy = WasmHandler {
        index,
        module: WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        },
    };
    let mut random = mod_api::LegacyRandomSource::new(0x5eed);
    let mut expected = mod_api::LegacyRandomSource::new(0x5eed);
    let mut event = mod_api::AnimateTickEvent {
        level: mod_api::LevelRef::new(mod_api::WorldEpoch::new(1), mod_api::DimensionKey::NETHER),
        pos: mod_api::BlockPos::new(5, 63, 4),
        state: mod_api::BlockStateRecord::new(REGISTERED_BLOCK.get().unwrap(), 0).into(),
        random: &mut random,
    };
    mod_api::BlockPart::animate_tick(&proxy, &mut Client, &mut event);
    let want = format!(
        "{} {} {} {}",
        expected.next_float(),
        expected.next_int_bound(10),
        expected.next_boolean(),
        expected.next_double()
    );
    assert_eq!(DRAWS.with_borrow(Clone::clone), [want]);
    assert_eq!(random.next_long(), expected.next_long());
}
