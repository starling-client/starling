//! Observable host query semantics through real holder writes and transaction journals.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{AttachmentQueries, AttachmentQueriesMut, AttachmentType, HostTransaction};
use sunrise_client::chunk::Chunk;

static VALUE: AttachmentType<u32> = AttachmentType {
    namespace: "query_test",
    name: "value",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
fn values(game: &Game<'_>) -> Vec<u32> {
    let host: &dyn mod_api::Host = game;
    let query = host.query(&VALUE).unwrap();
    let mut rows = query.iter();
    std::iter::from_fn(|| rows.try_next().unwrap())
        .map(|row| {
            row.with_value(game.attachment_frame_borrow(), |value| *value)
                .unwrap()
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn query_snapshot_dimension_and_stale_writes() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        assert!(values(game).is_empty());
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        VALUE.set(game, Holder::Chunk([2, 3]), 10).unwrap();
        let batch = game.query_mut(&VALUE).unwrap();
        let row = batch.iter().try_next().unwrap().unwrap();
        VALUE.set(game, Holder::Chunk([2, 3]), 11).unwrap();
        assert_eq!(
            row.with_value(game.attachment_frame_borrow(), |value| *value)
                .unwrap(),
            10
        );
        assert_eq!(values(game), [11]);
        game.enter_level(Dim::Nether).unwrap();
        assert!(values(game).is_empty());
        assert_eq!(
            row.set(game, 12),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        VALUE.set(game, Holder::Chunk([2, 3]), 20).unwrap();
        assert_eq!(values(game), [20]);
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(values(game), [11]);
        row.set(game, 12).unwrap();
        assert_eq!(values(game), [12]);
        let retained = game.world.chunks.remove(&(2, 3)).unwrap();
        assert!(values(game).is_empty());
        assert_eq!(
            row.with_value(game.attachment_frame_borrow(), |value| *value)
                .unwrap(),
            10
        );
        assert_eq!(
            row.remove(game),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        game.world.chunks.insert((2, 3), retained);
        assert_eq!(
            row.set(game, 13),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        let fresh = game.query_mut(&VALUE).unwrap();
        let fresh_row = fresh.iter().try_next().unwrap().unwrap();
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        assert_eq!(
            fresh_row.set(game, 14),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        assert!(values(game).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn query_writes_join_transaction_journal_without_implicit_commit() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        VALUE.set(game, Holder::Chunk([2, 3]), 10).unwrap();
        let batch = game.query_mut(&VALUE).unwrap();
        let row = batch.iter().try_next().unwrap().unwrap();
        let aborted: Result<(), ()> = game.transaction(|host, _| {
            row.set(host, 11).unwrap();
            assert_eq!(values(host), [11]);
            let current = host.query_mut(&VALUE).unwrap();
            current
                .iter()
                .try_next()
                .unwrap()
                .unwrap()
                .remove(host)
                .unwrap();
            assert!(values(host).is_empty());
            Err(())
        });
        assert_eq!(aborted, Err(()));
        assert_eq!(values(game), [10]);
        assert_eq!(
            row.with_value(game.attachment_frame_borrow(), |value| *value)
                .unwrap(),
            10
        );
        let committed: Result<(), ()> = game.transaction(|host, _| {
            let current = host.query_mut(&VALUE).unwrap();
            current
                .iter()
                .try_next()
                .unwrap()
                .unwrap()
                .set(host, 15)
                .unwrap();
            Ok(())
        });
        assert_eq!(committed, Ok(()));
        drop(batch);
        assert_eq!(values(game), [15]);
    });
}

#[test]
#[ignore = "daily"]
fn rolled_back_attachment_queries_cannot_read_or_write_a_resident_holder() {
    use mod_api::AttachmentRowError::OutsideTransaction;

    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        let holder = Holder::Chunk([2, 3]);
        VALUE.set(game, holder, 10).unwrap();
        let before = game.query_mut(&VALUE).unwrap();
        let before_row = before.iter().try_next().unwrap().unwrap();
        let scoped_holder = before_row.holder();

        // Use the raw seal so the test can keep an iterator and both kinds of row,
        // as well as their batches, alive across rollback.
        let seal = game.transaction_open();
        VALUE.set(game, holder, 11).unwrap();
        let reader = game.query(&VALUE).unwrap();
        let writer = game.query_mut(&VALUE).unwrap();
        let mut retained_iter = reader.iter();
        let read_row = retained_iter.try_next().unwrap().unwrap();
        let write_row = writer.iter().try_next().unwrap().unwrap();
        assert_eq!(
            read_row.with_value(Default::default(), |value| {
                game.transaction_close(&seal, false);
                *value
            }),
            Err(OutsideTransaction)
        );
        let mut foreign = sunrise_mod_runtime::attachments::AttachmentQueryScopes::default();
        foreign.enter();
        assert_eq!(
            write_row.with_value(
                mod_api::__private::game::attachment_read_scope(&foreign),
                |value| {
                    game.transaction_close(&seal, false);
                    *value
                },
            ),
            Err(OutsideTransaction)
        );
        let host: &dyn mod_api::ServerHost = game;
        let scope = host.attachment_read_scope().unwrap();
        assert_eq!(read_row.with_value(scope, |value| *value), Ok(11));
        assert_eq!(write_row.with_value(scope, |value| *value), Ok(11));
        assert_eq!(reader.get(scope, scoped_holder), Ok(Some(11)));
        assert_eq!(
            reader.get(
                mod_api::__private::game::attachment_read_scope(&foreign),
                scoped_holder,
            ),
            Err(OutsideTransaction)
        );
        let child = game.transaction_open();
        assert_eq!(
            read_row.with_value(game.attachment_frame_borrow(), |value| *value),
            Err(OutsideTransaction)
        );
        game.transaction_close(&child, false);
        assert_eq!(
            read_row.with_value(game.attachment_frame_borrow(), |value| *value),
            Ok(11)
        );
        game.transaction_close(&seal, false);

        assert_eq!(values(game), [10]);
        assert_eq!(
            reader.get(game.attachment_frame_borrow(), scoped_holder),
            Err(OutsideTransaction)
        );
        assert!(matches!(retained_iter.try_next(), Err(OutsideTransaction)));
        assert!(matches!(writer.iter().try_next(), Err(OutsideTransaction)));
        assert_eq!(
            read_row.with_value(game.attachment_frame_borrow(), |value| *value),
            Err(OutsideTransaction)
        );
        assert_eq!(
            write_row.with_value(game.attachment_frame_borrow(), |value| *value),
            Err(OutsideTransaction)
        );
        assert_eq!(write_row.set(game, 99), Err(OutsideTransaction));
        assert_eq!(write_row.remove(game), Err(OutsideTransaction));
        assert_eq!(values(game), [10]);
        before_row.set(game, 12).unwrap();
        assert_eq!(values(game), [12]);
    });
}
