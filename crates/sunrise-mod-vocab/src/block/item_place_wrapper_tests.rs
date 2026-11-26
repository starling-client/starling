use super::*;
use crate::prelude::ItemsHost;
use crate::testing::FakeHost;
use crate::{HostTransaction, Identifier, Tag};

const STEVE: u128 = 7;

/// A host with Steve online and `ids` registered, each stacking to its count, and their
/// resources.
fn host_with(items: &[(&str, i32)]) -> (FakeHost, Vec<ItemResource>) {
    let mut host = FakeHost::new();
    host.join(STEVE, 1);
    let ids: Vec<_> = items
        .iter()
        .map(|(id, _)| Identifier::parse(id).unwrap())
        .collect();
    host.registries.resolve_items(ids.clone()).unwrap();
    for (id, stack) in items {
        host.registries
            .item_component(id, "minecraft:max_stack_size", Tag::Int(*stack));
    }
    let resources = ids
        .iter()
        .map(|id| ItemResource::of(&ItemStack::new(host.item_key(id).unwrap(), 1).unwrap()))
        .collect();
    (host, resources)
}

fn infinite_hand() -> ItemPlace {
    ItemPlace::InfiniteMaterials {
        player: STEVE,
        inner: Box::new(ItemPlace::PlayerHand {
            player: STEVE,
            hand: Hand::Main,
        }),
    }
}

/// `place.insert` of `amount` in a transaction that commits.
fn insert(host: &mut FakeHost, place: &ItemPlace, resource: &ItemResource, amount: u32) -> u32 {
    let host: &mut dyn ServerHost = host;
    host.transaction(|host, tx| place.insert(host, tx, resource, amount))
        .unwrap()
}

fn extract(host: &mut FakeHost, place: &ItemPlace, resource: &ItemResource, amount: u32) -> u32 {
    let host: &mut dyn ServerHost = host;
    host.transaction(|host, tx| place.extract(host, tx, resource, amount))
        .unwrap()
}

/// What Steve holds, by slot and count.
fn slots(host: &FakeHost) -> Vec<(i32, u32)> {
    let steve = host.players.get(STEVE).unwrap();
    steve
        .slots
        .iter()
        .map(|(&slot, s)| (slot, s.count()))
        .collect()
}

/// What the game gave Steve beyond his slots, as `Player.drop` would when the inventory is full.
fn given(host: &FakeHost) -> Vec<u32> {
    let steve = host.players.get(STEVE).unwrap();
    steve.given.iter().map(ItemStack::count).collect()
}

#[test]
fn infinite_materials_drops_the_one_when_the_inventory_is_full() {
    let (mut host, r) = host_with(&[("minecraft:water_bucket", 1), ("minecraft:dirt", 64)]);
    let (bucket, dirt) = (&r[0], &r[1]);
    let steve = host.players.get_mut(STEVE).unwrap();
    steve.infinite_materials = true;
    for slot in 0..36 {
        steve.slots.insert(slot, dirt.to_stack(64).unwrap());
    }
    // Taken whole; the one he has none of goes to the game's give-or-drop, not lost.
    assert_eq!(insert(&mut host, &infinite_hand(), bucket, 3), 3);
    assert_eq!(given(&host), [1]);
    assert_eq!(slots(&host).len(), 36);
    // Now he has one: nothing more is given.
    host.players
        .get_mut(STEVE)
        .unwrap()
        .slots
        .insert(5, bucket.to_stack(1).unwrap());
    assert_eq!(insert(&mut host, &infinite_hand(), bucket, 1), 1);
    assert_eq!(given(&host), [1]);
}

#[test]
fn infinite_materials_puts_the_one_in_an_empty_slot_and_gives_without_taking() {
    let (mut host, r) = host_with(&[("minecraft:water_bucket", 1), ("minecraft:dirt", 64)]);
    let (bucket, dirt) = (&r[0], &r[1]);
    let steve = host.players.get_mut(STEVE).unwrap();
    steve.infinite_materials = true;
    steve.slots.insert(0, dirt.to_stack(10).unwrap());
    assert_eq!(insert(&mut host, &infinite_hand(), bucket, 1), 1);
    // Slot 0 holds dirt, so the bucket goes where the inventory has room; the fake records it
    // as given, as `Inventory.add` places it.
    assert_eq!(given(&host), [1]);
    // An extract gives what the hand holds without taking it.
    assert_eq!(extract(&mut host, &infinite_hand(), dirt, 64), 10);
    assert_eq!(slots(&host), [(0, 10)]);
}

#[test]
fn inserting_nothing_into_infinite_materials_mints_nothing() {
    let (mut host, r) = host_with(&[("minecraft:water_bucket", 1)]);
    let bucket = &r[0];
    host.players.get_mut(STEVE).unwrap().infinite_materials = true;
    assert_eq!(insert(&mut host, &infinite_hand(), bucket, 0), 0);
    assert!(slots(&host).is_empty());
    assert!(given(&host).is_empty());
}

#[test]
fn without_infinite_materials_it_is_the_place_inside() {
    let (mut host, r) = host_with(&[("minecraft:dirt", 64)]);
    let dirt = &r[0];
    host.players
        .get_mut(STEVE)
        .unwrap()
        .slots
        .insert(0, dirt.to_stack(10).unwrap());
    assert_eq!(extract(&mut host, &infinite_hand(), dirt, 4), 4);
    assert_eq!(slots(&host), [(0, 6)]);
}

#[test]
fn wrappers_nest_far_deeper_than_any_stack_would_recurse() {
    let (mut host, r) = host_with(&[("minecraft:dirt", 64)]);
    let mut stored = r[0].to_stack(5).unwrap();
    let data = Tag::Compound([("value".into(), Tag::Int(42))].into_iter().collect());
    stored
        .set_components(
            [("minecraft:custom_data".into(), data.clone())]
                .into_iter()
                .collect(),
        )
        .unwrap();
    let dirt = ItemResource::of(&stored);
    host.players.get_mut(STEVE).unwrap().slots.insert(3, stored);
    let mut deep = ItemPlace::PlayerSlot {
        player: STEVE,
        slot: 3,
    };
    for depth in 0..100_000 {
        deep = match depth % 3 {
            0 => ItemPlace::Strict(Box::new(deep)),
            1 => ItemPlace::OneByOne(Box::new(deep)),
            _ => ItemPlace::InfiniteMaterials {
                player: STEVE,
                inner: Box::new(deep),
            },
        };
    }
    let server: &mut dyn ServerHost = &mut host;
    assert_eq!(deep.amount(server), 1);
    let (count, projected) = deep
        .with_stack(server, |view| (view.count(), view.project()))
        .unwrap();
    let projected = projected.unwrap();
    assert_eq!(count, 1);
    assert_eq!(projected.count(), 1);
    assert_eq!(
        projected
            .components()
            .unwrap()
            .as_nbt()
            .get(&crate::JavaString::from("minecraft:custom_data")),
        Some(&data),
    );
    assert_eq!(slots(&host), [(3, 5)], "lending one did not change storage");
    assert_eq!(extract(&mut host, &deep, &dirt, 5), 1);
    assert_eq!(slots(&host), [(3, 4)]);
    let cloned = deep.clone();
    assert_eq!(cloned.amount(&mut host), 1);
    drop(cloned);
    drop(deep);
}

#[test]
fn one_by_one_caps_both_inserts_and_extracts_at_one() {
    let (mut host, r) = host_with(&[("minecraft:dirt", 64)]);
    let dirt = &r[0];
    host.players
        .get_mut(STEVE)
        .unwrap()
        .slots
        .insert(3, dirt.to_stack(5).unwrap());
    let slot = ItemPlace::PlayerSlot {
        player: STEVE,
        slot: 3,
    };
    let one = ItemPlace::OneByOne(Box::new(slot));
    assert_eq!(insert(&mut host, &one, dirt, 4), 1);
    assert_eq!(slots(&host), [(3, 6)]);
    assert_eq!(extract(&mut host, &one, dirt, 4), 1);
    assert_eq!(slots(&host), [(3, 5)]);
}

/// A mod's multiblock handler: whichever block of the multiblock it was reached from, index `i`
/// is slot `i` of the controller's container.
struct Multiblock {
    level: LevelRef,
    controller: [i32; 3],
}

impl ResourceHandler<ItemResource> for Multiblock {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        2
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
        None
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&ItemResource>) -> u32 {
        0
    }
    fn insert_at(
        &self,
        _: &mut dyn ServerHost,
        _: &TransactionContext,
        _: usize,
        _: &ItemResource,
        _: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        Ok(0)
    }
    fn extract_at(
        &self,
        _: &mut dyn ServerHost,
        _: &TransactionContext,
        _: usize,
        _: &ItemResource,
        _: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        Ok(0)
    }
    fn backing(&self, _: &mut dyn ServerHost, index: usize) -> Option<StorageSlot> {
        (index < 2).then_some(StorageSlot {
            level: self.level,
            pos: self.controller,
            slot: index,
        })
    }
}

/// The same handler without a `backing` answer.
struct Silent(Multiblock);

impl ResourceHandler<ItemResource> for Silent {
    fn size(&self, host: &mut dyn ServerHost) -> usize {
        self.0.size(host)
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        self.0.resource(host, index)
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.0.amount(host, index)
    }
    fn capacity(
        &self,
        host: &mut dyn ServerHost,
        index: usize,
        resource: Option<&ItemResource>,
    ) -> u32 {
        self.0.capacity(host, index, resource)
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        self.0.insert_at(host, tx, index, resource, amount)
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        self.0.extract_at(host, tx, index, resource, amount)
    }
}

#[test]
fn a_handler_reports_one_backing_from_every_block_of_a_multiblock() {
    let mut fake = FakeHost::new();
    let level = fake.level.level();
    let host: &mut dyn ServerHost = &mut fake;
    // Two blocks of one multiblock each answer with their own handler value.
    let from_a = Multiblock {
        level,
        controller: [0, 64, 0],
    };
    let from_b = Multiblock {
        level,
        controller: [0, 64, 0],
    };
    let a = from_a.backing(host, 1).unwrap();
    assert_eq!(a, from_b.backing(host, 1).unwrap());
    assert_ne!(a, from_a.backing(host, 0).unwrap());
    assert_eq!(
        a,
        StorageSlot {
            level,
            pos: [0, 64, 0],
            slot: 1
        }
    );
    assert_eq!(from_a.backing(host, 2), None);
}

#[test]
fn a_handler_that_does_not_override_backing_reports_none() {
    let mut fake = FakeHost::new();
    let level = fake.level.level();
    let host: &mut dyn ServerHost = &mut fake;
    let silent = Silent(Multiblock {
        level,
        controller: [0, 64, 0],
    });
    assert_eq!(silent.backing(host, 0), None);
}

#[test]
fn a_block_entity_place_reports_the_slot_wrapped_or_not() {
    let mut fake = FakeHost::new();
    let level = fake.level.level();
    fake.level.block_entity([3, 64, 3]).with_items(4);
    let host: &mut dyn ServerHost = &mut fake;
    let place = ItemPlace::BlockEntity {
        level,
        pos: [3, 64, 3],
        side: Some(Direction::Up),
        slot: 2,
    };
    let expected = Some(StorageSlot {
        level,
        pos: [3, 64, 3],
        slot: 2,
    });
    assert_eq!(place.backing(host), expected);
    let strict = ItemPlace::Strict(Box::new(place));
    assert_eq!(strict.backing(host), expected);
    let one = ItemPlace::OneByOne(Box::new(strict));
    assert_eq!(one.backing(host), expected);
    // A slot past the container, and a block with no container, have none.
    let past = ItemPlace::BlockEntity {
        level,
        pos: [3, 64, 3],
        side: None,
        slot: 4,
    };
    assert_eq!(past.backing(host), None);
    let none = ItemPlace::BlockEntity {
        level,
        pos: [9, 64, 9],
        side: None,
        slot: 0,
    };
    assert_eq!(none.backing(host), None);
}

#[test]
fn places_that_are_not_block_slots_have_no_backing() {
    let (mut fake, _) = host_with(&[]);
    let level = fake.level.level();
    fake.level.block_entity([3, 64, 3]).with_items(4);
    let host: &mut dyn ServerHost = &mut fake;
    let slot = ItemPlace::PlayerSlot {
        player: STEVE,
        slot: 0,
    };
    let hand = ItemPlace::PlayerHand {
        player: STEVE,
        hand: Hand::Main,
    };
    assert_eq!(slot.backing(host), None);
    assert_eq!(hand.backing(host), None);
    assert_eq!(infinite_hand().backing(host), None);
    // Even around a block slot, a player with infinite materials reaches it as a wrapper.
    let wrapped = ItemPlace::InfiniteMaterials {
        player: STEVE,
        inner: Box::new(ItemPlace::BlockEntity {
            level,
            pos: [3, 64, 3],
            side: None,
            slot: 0,
        }),
    };
    assert_eq!(wrapped.backing(host), None);
}
