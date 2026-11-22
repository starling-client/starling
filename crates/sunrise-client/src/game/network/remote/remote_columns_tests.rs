use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::{chunk::Chunk, containers::BlockEntity};

#[test]
#[ignore = "daily"]
fn forgetting_and_closing_remote_columns_clear_their_data() {
    with_game(|game| {
        for x in 0..2 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, 0));
            game.world
                .block_entities
                .insert((x * 16, 64, 0), BlockEntity::Chest(vec![]));
        }
        // A same-position resend replaces the data without removing the column.
        game.streaming
            .replace_remote(&mut game.world, Chunk::empty(0, 0));
        assert_eq!(game.world.chunks.len(), 2);
        assert!(game.world.chunks.contains_key(&(0, 0)));
        assert!(game.streaming.is_lit(0, 0));
        let mut forgotten = [(0, 0)].into_iter().collect();
        game.forget_remote_columns(&mut forgotten, true);
        assert!(!game.world.chunks.contains_key(&(0, 0)));
        assert!(!game.world.block_entities.contains_key(&(0, 64, 0)));
        assert!(!game.streaming.is_lit(0, 0));
        assert!(game.world.chunks.contains_key(&(1, 0)));
        game.forget_remote_columns(&mut [(0, 0)].into_iter().collect(), true);
        for x in [7, 13] {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, 0));
            game.world
                .block_entities
                .insert((x * 16, 64, 0), BlockEntity::Chest(vec![]));
        }
        // Cache-centre packet: move to (15, 0), at radius 10.
        game.drop_remote_chunks_out_of_range((15, 0), 10);
        assert!(!game.world.chunks.contains_key(&(1, 0)));
        assert!(game.world.chunks.contains_key(&(7, 0)));
        assert!(game.world.chunks.contains_key(&(13, 0)));
        // Cache-radius packet: shrinking to 2 keeps radius 5 around (15, 0).
        game.drop_remote_chunks_out_of_range((15, 0), 2);
        assert!(!game.world.chunks.contains_key(&(7, 0)));
        assert!(game.world.chunks.contains_key(&(13, 0)));
        assert!(!game.world.block_entities.contains_key(&(7 * 16, 64, 0)));
        // A later forget for an evicted column is harmless.
        game.forget_remote_columns(&mut [(7, 0)].into_iter().collect(), true);
        game.forget_all_remote_columns();
        assert!(game.world.chunks.is_empty());
        assert!(game.world.block_entities.is_empty());
    });
}

/// The client chunk seam (game/network/remote_chunks.rs) across every free
/// route, as the freed set each route returns: a forget packet, the
/// eager centre-move drop (a cache-centre packet), the radius-shrink
/// drop (a cache-radius packet) and the client level going. One freed
/// entry per column, in the order the routes free them, and a repeat
/// forget for an already-freed column frees nothing: adopting a column
/// is a chunk packet's doing (`remote_chunk`), not the streaming
/// level's.
#[test]
#[ignore = "daily"]
fn every_free_route_returns_the_columns_it_freed_once() {
    with_game(|game| {
        for x in [0, 1] {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, 0));
        }
        // A forget packet frees its column once; the same forget again,
        // for a column already freed, frees nothing.
        let mut gone = [(0, 0)].into_iter().collect();
        assert_eq!(game.forget_remote_columns(&mut gone, true), [(0, 0)]);
        let mut gone = [(0, 0)].into_iter().collect();
        assert!(game.forget_remote_columns(&mut gone, true).is_empty());
        for x in [7, 13] {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, 0));
        }
        // The centre move to (15, 0) at radius 10 frees (1, 0) only.
        assert_eq!(game.drop_remote_chunks_out_of_range((15, 0), 10), [(1, 0)]);
        // Shrinking the radius to 2 frees (7, 0) only.
        assert_eq!(game.drop_remote_chunks_out_of_range((15, 0), 2), [(7, 0)]);
        // The client level going frees what is left.
        assert_eq!(game.forget_all_remote_columns(), [(13, 0)]);
        assert!(game.world.chunks.is_empty());
        assert!(game.world.block_entities.is_empty());
    });
}

/// The seam sits at the packet level: streaming adopting, replacing or
/// re-sending a column keeps it resident — a re-sent position loads
/// again (`remote_chunk`), never an unload and a load — so a forget
/// returns exactly it.
#[test]
#[ignore = "daily"]
fn streaming_adoption_and_resend_stay_resident_until_the_forget() {
    with_game(|game| {
        game.streaming
            .insert_remote(&mut game.world, Chunk::empty(0, 0));
        // A re-send of a resident, lit column takes replace_remote.
        game.streaming
            .replace_remote(&mut game.world, Chunk::empty(0, 0));
        assert!(game.world.chunks.contains_key(&(0, 0)));
        let mut gone = [(0, 0)].into_iter().collect();
        assert_eq!(game.forget_remote_columns(&mut gone, true), [(0, 0)]);
    });
}

#[test]
#[ignore = "daily"]
fn a_forget_packet_keeps_published_light_beside_a_loaded_column() {
    use sunrise_protocol::packets::play::{Clientbound, ForgetLevelChunk};
    with_game(|game| {
        use sunrise_client::chunk::SECTIONS;
        for x in 0..2 {
            let mut chunk = Chunk::empty(x, 0);
            chunk.set(0, 0, 0, 1);
            let s = chunk.section_index(0).unwrap();
            chunk.add_light_support(s);
            let mut layers = vec![(None, None); SECTIONS + 2];
            layers[s + 1] = (Some(vec![0x66; 2048]), Some(vec![0x99; 2048]));
            sunrise_client::light::light_from_layers(&mut chunk, &layers, true);
            game.streaming.insert_remote(&mut game.world, chunk);
        }
        game.apply_packet(Clientbound::ForgetLevelChunk(ForgetLevelChunk {
            pos: [1, 0],
        }))
        .unwrap();
        assert!(!game.world.loaded(16, 0));
        assert_eq!(game.world.light(16, 0, 0), 0x69);
        game.apply_packet(Clientbound::ForgetLevelChunk(ForgetLevelChunk {
            pos: [0, 0],
        }))
        .unwrap();
        assert_eq!(game.world.light(16, 0, 0), 0xF0);
    });
}
