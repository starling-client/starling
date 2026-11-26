/// Install a part on glass that records each `on_place` it hears and, when a neighbour gives a
/// signal (`has_neighbor_signal`), turns itself into white stained glass in the same call, for
/// real-game tests.
pub fn block_on_place_game_thread() {
    struct Lamp {
        lit: BlockKey,
    }
    #[mod_api::sunrise_part]
    impl BlockPart for Lamp {
        fn on_place(&self, level: &mut dyn crate::LevelWriteHost, event: &mut crate::OnPlaceEvent) {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            ON_PLACES.with(|r| {
                r.borrow_mut()
                    .push((pos, event.old_state.block(), event.moved_by_piston))
            });
            if level.has_neighbor_signal(pos).expect("a level view") {
                let lit = BlockState::of(&level.block_states(), self.lit, &[]).unwrap();
                assert!(level.set_block(pos, lit, BlockUpdateFlags::CLIENTS));
            }
        }
    }
    struct LampMod;
    #[mod_api::__subscriptions]
    impl Hooks for LampMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            let lamp = Lamp {
                lit: r.block_key("minecraft:white_stained_glass"),
            };
            r.block_part(glass, lamp);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("block_on_place_test", Box::new(LampMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}

std::thread_local! {
    static ON_PLACES: std::cell::RefCell<Vec<([i32; 3], BlockKey, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The `on_place` calls the lamp glass heard on this thread, taken: its position, the block it
/// replaced and `moved_by_piston`.
pub fn block_on_places() -> Vec<([i32; 3], BlockKey, bool)> {
    ON_PLACES.with(|r| std::mem::take(&mut *r.borrow_mut()))
}
