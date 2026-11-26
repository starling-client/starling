/// Install a part on glass that records each `neighbor_changed` it hears and, when the
/// changed block is a lever, turns itself into white stained glass with `Block.UPDATE_ALL` in the
/// same call, as `RedstoneLampBlock.neighborChanged` lights itself, for real-game tests.
pub fn neighbor_writer_game_thread() {
    struct Writer {
        lever: BlockKey,
        lit: BlockKey,
    }
    #[mod_api::sunrise_part]
    impl BlockPart for Writer {
        fn neighbor_changed(
            &self,
            level: &mut dyn crate::LevelWriteHost,
            event: &mut crate::NeighborChangedEvent,
        ) {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            NEIGHBOR_WRITES.with(|r| r.borrow_mut().push((pos, event.changed)));
            if event.changed == self.lever {
                let lit = BlockState::of(&level.block_states(), self.lit, &[]).unwrap();
                assert!(level.set_block(
                    pos,
                    lit,
                    BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS
                ));
            }
        }
    }
    struct WriterMod;
    #[mod_api::__subscriptions]
    impl Hooks for WriterMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            let writer = Writer {
                lever: r.block_key("minecraft:lever"),
                lit: r.block_key("minecraft:white_stained_glass"),
            };
            r.block_part(glass, writer);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("neighbor_writer_test", Box::new(WriterMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}

std::thread_local! {
    static NEIGHBOR_WRITES: std::cell::RefCell<Vec<([i32; 3], BlockKey)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The `neighbor_changed` calls the writing glass heard on this thread, taken: its position
/// and the changed block.
pub fn neighbor_writes() -> Vec<([i32; 3], BlockKey)> {
    NEIGHBOR_WRITES.with(|r| std::mem::take(&mut *r.borrow_mut()))
}
