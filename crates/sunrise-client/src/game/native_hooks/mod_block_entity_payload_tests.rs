#![cfg(test)]

#[test]
#[ignore = "daily"]
fn replacing_mod_block_entity_payload_keeps_its_type_and_saved_identity() {
    use super::super::native_entity_refs::tests::with_game;
    use std::sync::Arc;
    use sunrise_client::chunk::Chunk;
    use sunrise_client::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields};
    use sunrise_client::save::{block_entity_encode, decode_block_entity};

    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let pos = (1, 64, 1);
        let machine = ModBlockEntity::new("test:machine".into(), Default::default());
        let made = machine.revision();
        game.world
            .block_entities
            .insert(pos, BlockEntity::Mod(Box::new(machine)));
        assert_eq!(
            game.native_with_mod_block_entity([1, 64, 1], |entity| {
                **entity = ModBlockEntityFields {
                    data: vec![7],
                    ..Default::default()
                };
                assert_eq!(entity.id(), "test:machine");
            }),
            Some(())
        );
        let entity = game.world.block_entities.get(&pos).unwrap();
        let BlockEntity::Mod(machine) = entity else {
            panic!("not a mod block entity");
        };
        assert_eq!(machine.id(), "test:machine");
        assert_eq!(machine.data, [7]);
        assert_ne!(machine.revision(), made);
        assert!(game.world.modified.contains(&(0, 0)));
        let saved =
            block_entity_encode::encode(pos, entity, "minecraft:stone", &Default::default());
        let (saved_pos, loaded) = decode_block_entity(&saved).unwrap();
        assert_eq!(saved_pos, pos);
        assert_eq!(loaded.id(), "test:machine");
    });
}
