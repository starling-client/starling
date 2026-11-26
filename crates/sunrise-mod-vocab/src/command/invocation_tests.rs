//! `CommandInvocation::player()` follows `entity`, and its wire record refuses a player that
//! is not the invocation's entity.
use super::{CommandArgument, CommandArgumentValue, CommandInvocation, InvocationRecord};
use crate::wire::{Reader, Writer};
use crate::{DimensionKey, EntityRef, PlayerRef, WorldEpoch};

fn alex() -> PlayerRef {
    PlayerRef {
        entity: EntityRef::new(WorldEpoch::new(3), 12),
        uuid: 0xa1e8,
    }
}

fn invocation() -> CommandInvocation {
    CommandInvocation::new(
        vec!["party".into(), "leave".into()],
        vec![CommandArgument::new(
            "count".into(),
            CommandArgumentValue::Integer(3),
        )],
        DimensionKey::OVERWORLD,
        [0.5, 64.0, -2.5],
        [30.0, -10.0],
    )
}

fn encoded<T: crate::wire::Wire>(value: &T) -> Vec<u8> {
    let mut out = Writer::new();
    out.put(value);
    assert_eq!(out.error(), None);
    out.bytes().to_vec()
}

#[test]
fn player_follows_a_changed_entity() {
    let mob = EntityRef::new(WorldEpoch::new(3), 40);
    let command = invocation().with_player(alex());
    assert_eq!(command.entity, Some(alex().entity()));
    assert_eq!(command.player(), Some(alex()));

    let mut cleared = command.clone();
    cleared.entity = None;
    assert_eq!(cleared.player(), None);

    let mut replaced = command.clone();
    replaced.entity = Some(mob);
    assert_eq!(replaced.player(), None);

    // Put back, the entity is Alex's again.
    replaced.entity = Some(alex().entity());
    assert_eq!(replaced.player(), Some(alex()));

    let as_mob = command.with_entity(mob);
    assert_eq!((as_mob.entity, as_mob.player()), (Some(mob), None));
}

#[test]
fn invocations_round_trip_through_wire() {
    let mob = EntityRef::new(WorldEpoch::new(3), 40);
    for command in [
        invocation(),
        invocation().with_entity(mob),
        invocation().with_player(alex()),
    ] {
        let decoded = Reader::decode::<CommandInvocation>(&encoded(&command)).unwrap();
        assert_eq!(decoded.player(), command.player());
        assert_eq!(decoded, command);
    }
}

#[test]
fn a_record_whose_player_is_not_its_entity_does_not_decode() {
    let mob = EntityRef::new(WorldEpoch::new(3), 40);
    for entity in [Some(mob), None] {
        let mut record = InvocationRecord::from(&invocation());
        record.entity = entity;
        record.player = Some(alex());
        assert!(Reader::decode::<CommandInvocation>(&encoded(&record)).is_err());
    }
}

#[test]
fn a_changed_entity_encodes_without_its_old_player() {
    let mob = EntityRef::new(WorldEpoch::new(3), 40);
    let mut changed = invocation().with_player(alex());
    changed.entity = Some(mob);
    let mut cleared = invocation().with_player(alex());
    cleared.entity = None;
    for command in [changed, cleared] {
        let decoded = Reader::decode::<CommandInvocation>(&encoded(&command)).unwrap();
        assert_eq!((decoded.entity, decoded.player()), (command.entity, None));
    }
}
