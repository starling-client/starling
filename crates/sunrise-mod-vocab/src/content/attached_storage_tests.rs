//! The handlers and their codecs, against the fixture host: literal amounts on writes, the
//! hostile saved shapes refused, and the journal's nested snapshots.
use super::*;
use crate::{
    AttachmentDescriptor, Capability, HostTransaction, ItemKey, resource_util, testing::FakeHost,
};
use std::cell::RefCell;

thread_local! {
    /// The one item `accepts_the_chosen` lets in: a `fn` pointer cannot capture it.
    static CHOSEN: RefCell<Option<ItemKey>> = const { RefCell::new(None) };
}
/// A `valid` the tests can point a handler at: `true` for the item `CHOSEN` holds.
fn accepts_the_chosen(resource: &ItemResource) -> bool {
    CHOSEN.with_borrow(|chosen| {
        let Some(chosen) = chosen else {
            return false;
        };
        resource
            .to_stack(1)
            .is_some_and(|stack| stack.item() == *chosen)
    })
}

fn install_items() -> AttachmentKey<StackList<ItemResource>> {
    crate::parts::install_attachment_fixture(
        "attached_storage_test",
        AttachmentDescriptor::new("attached_storage_test:items", StackList::new)
            .serialize(ITEM_STACKS),
    )
}
fn install_valid_items() -> AttachmentKey<StackList<ItemResource>> {
    crate::parts::install_attachment_fixture(
        "attached_storage_test",
        AttachmentDescriptor::new("attached_storage_test:valid_items", StackList::new)
            .serialize(ITEM_STACKS),
    )
}
fn install_fluids() -> AttachmentKey<StackList<FluidResource>> {
    crate::parts::install_attachment_fixture(
        "attached_storage_test",
        AttachmentDescriptor::new("attached_storage_test:fluids", StackList::new)
            .serialize(FLUID_STACKS),
    )
}
fn install_energy() -> AttachmentKey<u32> {
    crate::parts::install_attachment_fixture(
        "attached_storage_test",
        AttachmentDescriptor::new("attached_storage_test:energy", || 0u32),
    )
}
const CHUNK: Holder = Holder::Chunk([2, 3]);

/// Complete the fixture load's tables with `items` and `fluids`, so the stacks below resolve.
fn resolve(items: &[&str], fluids: &[&str]) {
    let mut host = FakeHost::new();
    if !items.is_empty() {
        host.registries
            .resolve_items(
                items
                    .iter()
                    .map(|id| Identifier::parse(id).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
    }
    if !fluids.is_empty() {
        host.registries
            .resolve_fluids(fluids.iter().copied())
            .unwrap();
    }
}

/// The stack of the resolved item `id`, one of it.
fn item(id: &str) -> ItemResource {
    let key = item_key_by_identifier(&Identifier::parse(id).unwrap()).unwrap();
    ItemResource::of(&ItemStack::new(key, 1).unwrap())
}

/// One of the resolved fluid `id`.
fn fluid(id: &str) -> FluidResource {
    let key = crate::content::registry_keys::fluid_key(id).unwrap();
    FluidResource::of(&FluidStack {
        fluid: FluidIdentity::Resolved(key),
        amount: 1,
        components: None,
    })
}

fn in_world(test: impl FnOnce()) {
    test();
}

/// The water amount the fluids codec saves and reads back, with its index.
#[test]
fn a_fluid_tank_round_trips_through_its_codec_and_moves() {
    in_world(|| {
        let fluids = install_fluids();
        resolve(&[], &["minecraft:water"]);
        let mut host = FakeHost::new();
        let water = fluid("minecraft:water");
        let tanks = AttachedFluids::new(&fluids, CHUNK, 6000, 1000, |_| true);
        let moved = host.transaction(|host, tx| tanks.insert_at(host, tx, 5999, &water, 250));
        assert_eq!(moved, Ok(250));
        assert_eq!(tanks.amount(&mut host, 5999), 250);
        // The codec of the saved attachment reads back to the same index and amount.
        let server: &mut dyn ServerHost = &mut host;
        let held = fluids
            .get_existing(server, CHUNK, |value| value.cloned())
            .unwrap()
            .unwrap();
        let saved = (FLUID_STACKS.encode)(&held).unwrap();
        let reloaded = (FLUID_STACKS.decode)(&saved).unwrap();
        assert_eq!(reloaded, StackList::from([(5999, (water.clone(), 250))]));
        // A transfer takes the same amount out again.
        let came = host.transaction(|host, tx| tanks.extract_at(host, tx, 5999, &water, 250));
        assert_eq!(came, Ok(250));
        assert_eq!(tanks.amount(&mut host, 5999), 0);
    });
}

/// Extraction of a part of what an index holds, and none of another resource.
#[test]
fn extraction_takes_part_and_none_of_another_resource() {
    in_world(|| {
        let items = install_items();
        resolve(&["minecraft:dirt", "minecraft:cobblestone"], &[]);
        let mut host = FakeHost::new();
        let dirt = item("minecraft:dirt");
        let cobble = item("minecraft:cobblestone");
        let slots = AttachedItems::new(&items, CHUNK, 9, 64, |_| true);
        let went = host.transaction(|host, tx| slots.insert_at(host, tx, 4, &dirt, 30));
        assert_eq!(went, Ok(30));
        let none = host.transaction(|host, tx| slots.extract_at(host, tx, 4, &cobble, 30));
        assert_eq!(none, Ok(0), "none of another resource");
        let came = host.transaction(|host, tx| slots.extract_at(host, tx, 4, &dirt, 12));
        assert_eq!(came, Ok(12));
        assert_eq!(slots.amount(&mut host, 4), 18);
        let rest = host.transaction(|host, tx| slots.extract_at(host, tx, 4, &dirt, 50));
        assert_eq!(rest, Ok(18), "at most what remains");
        let empty = host.transaction(|host, tx| slots.extract_at(host, tx, 4, &dirt, 1));
        assert_eq!(empty, Ok(0));
        assert_eq!(slots.amount(&mut host, 4), 0);
    });
}

/// `capacity` is the physical limit of an in-range index, `is_valid` the permission, and
/// extraction independent of validity.
#[test]
fn capacity_stays_physical_while_validity_refuses_inserts_only() {
    in_world(|| {
        let valid_items = install_valid_items();
        resolve(&["minecraft:dirt", "minecraft:cobblestone"], &[]);
        let mut host = FakeHost::new();
        let dirt = item("minecraft:dirt");
        let cobble = item("minecraft:cobblestone");
        CHOSEN.with_borrow_mut(|chosen| *chosen = Some(dirt.to_stack(1).unwrap().item()));
        let only_dirt = AttachedItems::new(&valid_items, CHUNK, 4, 500, accepts_the_chosen);
        assert_eq!(
            only_dirt.capacity(&mut host, 2, Some(&cobble)),
            64,
            "the physical limit (the item's stack size), though cobblestone is not valid here"
        );
        assert!(!only_dirt.is_valid(&mut host, 2, &cobble));
        let refused = host.transaction(|host, tx| only_dirt.insert_at(host, tx, 2, &cobble, 10));
        assert_eq!(refused, Ok(0));
        let went = host.transaction(|host, tx| only_dirt.insert_at(host, tx, 2, &dirt, 120));
        assert_eq!(went, Ok(64), "at most the item's stack size per index");
        // A cobblestone another (permissive) handler's write stored reads out through this one.
        let anything = AttachedItems::new(&valid_items, CHUNK, 4, 500, |_| true);
        let _ = host.transaction(|host, tx| anything.insert_at(host, tx, 3, &cobble, 7));
        let came = host.transaction(|host, tx| only_dirt.extract_at(host, tx, 3, &cobble, 7));
        assert_eq!(came, Ok(7), "extraction is independent of validity");
    });
}

/// The per-call limits and a nested abort: the inner transaction's write is undone, the
/// outer's stays.
#[test]
fn energy_limits_and_a_nested_abort() {
    in_world(|| {
        let energy = install_energy();
        resolve(&[], &[]);
        let mut host = FakeHost::new();
        let battery = AttachedEnergy::new(&energy, CHUNK, 100, 30, 20);
        let past_insert = host.transaction(|host, tx| battery.insert(host, tx, 150));
        assert_eq!(past_insert, Ok(30), "at most max_insert per call");
        let past_extract = host.transaction(|host, tx| battery.extract(host, tx, 25));
        assert_eq!(past_extract, Ok(20), "at most max_extract per call");
        let nested = host.transaction(|outer_host, outer_tx| {
            let aborted = outer_host.transaction(|host, tx| {
                assert_eq!(battery.insert(host, tx, 10), Ok(10));
                Err::<(), _>(())
            });
            assert_eq!(aborted, Err(crate::TransactionError::Aborted(())));
            let came = battery.extract(&mut *outer_host, outer_tx, 5);
            assert_eq!(came, Ok(5));
            came
        });
        assert_eq!(nested, Ok(5));
        assert_eq!(
            battery.amount(&mut host),
            5,
            "the inner write was undone, the outer's stayed"
        );
    });
}

/// A write through a smaller view keeps the higher indices a bigger one stored.
#[test]
fn a_smaller_view_keeps_higher_indices() {
    in_world(|| {
        let items = install_items();
        resolve(&["minecraft:dirt"], &[]);
        let mut host = FakeHost::new();
        let dirt = item("minecraft:dirt");
        let big = AttachedItems::new(&items, CHUNK, 9, 64, |_| true);
        let went = host.transaction(|host, tx| big.insert_at(host, tx, 7, &dirt, 12));
        assert_eq!(went, Ok(12));
        let small = AttachedItems::new(&items, CHUNK, 3, 64, |_| true);
        let went = host.transaction(|host, tx| small.insert_at(host, tx, 0, &dirt, 5));
        assert_eq!(went, Ok(5));
        assert_eq!(big.amount(&mut host, 7), 12, "the higher index survived");
        assert_eq!(big.amount(&mut host, 0), 5);
        assert_eq!(
            small.amount(&mut host, 7),
            0,
            "past the smaller view's size"
        );
    });
}

#[test]
fn saved_slots_use_the_nbt_int_range_without_expanding_gaps() {
    let last = i32::MAX as usize;
    let mut list = StackList::from([(last, ((), 1))]);
    let encode_entry = |_: &(), _: u32| Some(Compound::new());
    let mut saved = encode_stacks(&list, encode_entry).unwrap();
    assert_eq!(
        saved.get(&"stacks".into()),
        Some(&Tag::List(vec![Tag::compound([(
            "Slot",
            Tag::Int(i32::MAX),
        )])]))
    );
    assert_eq!(decode_stacks(&saved, |_| Some(((), 1))), Ok(list.clone()));

    list.insert(last + 1, ((), 1));
    assert_eq!(
        encode_stacks(&list, encode_entry),
        Err(AttachmentError::StackEncoding { index: last + 1 })
    );

    saved.insert(
        "stacks".into(),
        Tag::List(vec![Tag::compound([("Slot", Tag::Int(-1))])]),
    );
    assert_eq!(
        decode_stacks(&saved, |_| Some(((), 1))),
        Err(AttachmentError::Decode)
    );
}

#[test]
fn a_large_item_handler_keeps_the_saved_slot_format() {
    in_world(|| {
        let items = install_items();
        resolve(&["minecraft:dirt"], &[]);
        let mut host = FakeHost::new();
        let dirt = item("minecraft:dirt");
        let slots = AttachedItems::new(&items, CHUNK, 5001, 64, |_| true);
        assert_eq!(
            host.transaction(|host, tx| slots.insert_at(host, tx, 5000, &dirt, 1)),
            Ok(1)
        );
        assert_eq!(
            host.transaction(|host, tx| slots.insert_at(host, tx, 5001, &dirt, 1)),
            Ok(0)
        );
        let held = items
            .get_existing(&mut host as &mut dyn ServerHost, CHUNK, |value| {
                value.cloned()
            })
            .unwrap()
            .unwrap();
        let saved = (ITEM_STACKS.encode)(&held).unwrap();
        assert_eq!(
            saved.get(&"stacks".into()),
            Some(&Tag::List(vec![Tag::compound([
                ("Slot", Tag::Int(5000)),
                ("id", Tag::String("minecraft:dirt".into())),
                ("count", Tag::Int(1)),
            ])]))
        );
        assert_eq!(
            (ITEM_STACKS.decode)(&saved),
            Ok(StackList::from([(5000, (dirt, 1))]))
        );
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pebble {
    Slate,
    Quartz,
}

impl StackedResource for Pebble {}

fn install_memory() -> AttachmentKey<StackList<Pebble>> {
    crate::parts::install_attachment_fixture(
        "attached_storage_test",
        AttachmentDescriptor::new("attached_storage_test:memory", StackList::new),
    )
}

#[test]
fn an_unencoded_handler_bulk_extracts_in_order_without_walking_gaps() {
    in_world(|| {
        let memory = install_memory();
        let mut host = FakeHost::new();
        let slots = AttachedStacks::new(&memory, CHUNK, usize::MAX, 4, |_| true);
        let last = usize::MAX - 1;
        let slate = Pebble::Slate;
        assert_eq!(slots.size(&mut host), usize::MAX);
        assert_eq!(
            host.transaction(|host, tx| slots.extract(host, tx, &slate, 1)),
            Ok(0)
        );
        assert_eq!(
            host.transaction(|host, tx| Ok::<_, ()>(slots.insert_at(host, tx, last, &slate, 3))),
            Ok(Ok(3))
        );
        assert_eq!(
            host.transaction(|host, tx| Ok::<_, ()>(slots.insert_at(host, tx, 5, &slate, 1))),
            Ok(Ok(1))
        );
        assert_eq!(
            host.transaction(|host, tx| slots.extract(host, tx, &slate, 2)),
            Ok(2)
        );
        assert_eq!(
            memory.get_existing(&mut host as &mut dyn ServerHost, CHUNK, |value| value
                .cloned()),
            Ok(Some(StackList::from([(last, (slate, 2))])))
        );
        assert_eq!(
            host.transaction(|host, tx| slots.extract(host, tx, &slate, 10)),
            Ok(2)
        );
        assert_eq!(
            memory.get_existing(&mut host as &mut dyn ServerHost, CHUNK, |value| value
                .cloned()),
            Ok(None)
        );
    });
}

#[test]
fn bulk_insert_skips_a_sparse_range_that_rejects_the_resource() {
    in_world(|| {
        let memory = install_memory();
        let mut host = FakeHost::new();
        let slots = AttachedStacks::new(&memory, CHUNK, usize::MAX, 4, |r| *r == Pebble::Slate);
        assert_eq!(
            host.transaction(|host, tx| slots.insert(host, tx, &Pebble::Quartz, 1)),
            Ok(0)
        );
        assert_eq!(
            resource_util::insert_stacking(&mut host, &slots, &Pebble::Quartz, 1, None),
            Ok(0)
        );
        assert_eq!(
            memory.get_existing(&mut host as &mut dyn ServerHost, CHUNK, |value| value
                .cloned()),
            Ok(None)
        );
        let last = usize::MAX - 1;
        assert_eq!(
            host.transaction(|host, tx| Ok::<_, ()>(slots.insert_at(
                host,
                tx,
                last,
                &Pebble::Slate,
                3
            ))),
            Ok(Ok(3))
        );
        assert_eq!(
            resource_util::insert_stacking(&mut host, &slots, &Pebble::Slate, 2, None),
            Ok(2)
        );
        assert_eq!(slots.amount(&mut host, last), 4);
        assert_eq!(slots.amount(&mut host, 0), 1);
        assert_eq!(
            host.transaction(|host, tx| slots.insert(host, tx, &Pebble::Slate, 4)),
            Ok(4)
        );
        assert_eq!(slots.amount(&mut host, 0), 4);
        assert_eq!(slots.amount(&mut host, 1), 1);
    });
}

fn install_slot_one_only() -> AttachmentKey<StackList<Pebble>> {
    crate::parts::install_attachment_fixture(
        "attached_storage_test",
        AttachmentDescriptor::new("attached_storage_test:slot_one_only", StackList::new).serialize(
            AttachmentCodec {
                encode: |list: &StackList<Pebble>| {
                    if list.contains_key(&0) {
                        Err(AttachmentError::StackEncoding { index: 0 })
                    } else {
                        Ok(Compound::new())
                    }
                },
                decode: |_| Ok(StackList::new()),
            },
        ),
    )
}

#[test]
fn a_custom_codec_refusing_the_first_slot_still_takes_the_next() {
    in_world(|| {
        let slot_one_only = install_slot_one_only();
        let mut host = FakeHost::new();
        let slots = AttachedStacks::new(&slot_one_only, CHUNK, 4, 4, |_| true);
        assert_eq!(
            host.transaction(|host, tx| Ok::<_, ()>(slots.insert_at(
                host,
                tx,
                0,
                &Pebble::Slate,
                1
            ))),
            Ok(Ok(0))
        );
        assert_eq!(
            host.transaction(|host, tx| slots.insert(host, tx, &Pebble::Slate, 3)),
            Ok(3)
        );
        assert_eq!(slots.amount(&mut host, 0), 0);
        assert_eq!(slots.amount(&mut host, 1), 3);
    });
}

#[test]
fn bulk_insert_refuses_a_view_beyond_the_stock_codec() {
    let fluids = install_fluids();
    resolve(&[], &["minecraft:water"]);
    let mut host = FakeHost::new();
    let water = fluid("minecraft:water");
    let tanks = AttachedFluids::new(&fluids, CHUNK, usize::MAX, 1000, |_| true);
    let first_unencodable = i32::MAX as usize + 1;
    let range = first_unencodable..usize::MAX;
    let size = range.len();
    let view = resource_util::RangedResourceHandler::of(&mut host, Box::new(tanks), range).unwrap();
    assert_eq!(view.size(&mut host), size);
    assert_eq!(view.capacity(&mut host, 0, Some(&water)), 1000);
    assert_eq!(
        host.transaction(|host, tx| view.insert(host, tx, &water, 1)),
        Ok(0)
    );
    assert_eq!(
        fluids.get_existing(&mut host as &mut dyn ServerHost, CHUNK, |value| value
            .cloned()),
        Ok(None)
    );
}

#[test]
fn bulk_insert_skips_a_refused_attachment_and_still_visits_the_next_handler() {
    // A holder already storing a different type under the key refuses every write to it.
    let wrong_type = crate::parts::install_attachment_fixture(
        "attached_storage_test",
        AttachmentDescriptor::new("attached_storage_test:memory", || 0u32),
    );
    let mut host = FakeHost::new();
    wrong_type
        .set(&mut host as &mut dyn ServerHost, CHUNK, 7)
        .unwrap();
    let memory = install_memory();
    let bad = AttachedStacks::new(&memory, CHUNK, usize::MAX - 1, 4, |_| true);
    let combined = resource_util::CombinedResourceHandler::new(
        &mut host,
        vec![
            Box::new(bad),
            Box::new(resource_util::VoidingResourceHandler),
        ],
    );
    let view =
        resource_util::RangedResourceHandler::of(&mut host, Box::new(combined), 0..usize::MAX)
            .unwrap();
    assert_eq!(
        host.transaction(|host, tx| view.insert(host, tx, &Pebble::Slate, 1)),
        Ok(1)
    );
    let wrong_type = crate::parts::install_attachment_fixture(
        "attached_storage_test",
        AttachmentDescriptor::new("attached_storage_test:memory", || 0u32),
    );
    assert_eq!(
        wrong_type.get_existing(&mut host as &mut dyn ServerHost, CHUNK, |value| value
            .copied()),
        Ok(Some(7))
    );
}

#[test]
fn uniform_queries_keep_zero_capacity_separate_from_validity_through_wrappers() {
    let memory = install_memory();
    let mut host = FakeHost::new();
    let zero = AttachedStacks::new(&memory, CHUNK, usize::MAX - 1, 0, |r| *r == Pebble::Slate);
    let tail = AttachedStacks::single(&memory, CHUNK, 4, |r| *r == Pebble::Quartz);
    let combined = resource_util::CombinedResourceHandler::new(
        &mut host,
        vec![Box::new(zero), Box::new(tail)],
    );
    let combined = Capability::<dyn ResourceHandler<Pebble>> {
        handler: Box::new(combined),
        owner: Some("attached_storage_test"),
    };
    assert!(!resource_util::is_full(&mut host, &combined));
    assert!(resource_util::is_valid(
        &mut host,
        &combined,
        &Pebble::Quartz
    ));
    let zero =
        resource_util::RangedResourceHandler::of(&mut host, Box::new(combined), 1..usize::MAX - 1)
            .unwrap();
    assert!(resource_util::is_full(&mut host, &zero));
    assert!(resource_util::is_valid(&mut host, &zero, &Pebble::Slate));
    assert!(!resource_util::is_valid(&mut host, &zero, &Pebble::Quartz));
}

#[test]
fn bulk_moves_follow_sparse_source_order_through_wrappers() {
    use resource_util::{
        CombinedResourceHandler, RangedResourceHandler, move_first, move_resources,
    };

    in_world(|| {
        let (memory_record, memory) = crate::parts::registered_attachment_fixture(
            "attached_storage_test",
            AttachmentDescriptor::new("attached_storage_test:memory", StackList::new),
        );
        let (destination_record, destination_key) = crate::parts::registered_attachment_fixture(
            "attached_storage_test",
            AttachmentDescriptor::new("attached_storage_test:destination", StackList::new),
        );
        crate::parts::install_input(crate::parts::installation::InstalledInput {
            attachment_types: std::collections::HashMap::from([(
                "attached_storage_test",
                std::collections::HashMap::from([
                    ("memory", memory_record),
                    ("destination", destination_record),
                ]),
            )]),
            ..Default::default()
        });
        let mut host = FakeHost::new();
        let high = usize::MAX - 2;
        let source = AttachedStacks::new(&memory, CHUNK, usize::MAX, 4, |_| true);
        let destination = AttachedStacks::new(&destination_key, CHUNK, 4, 4, |_| true);
        for (index, resource, amount) in [
            (9, Pebble::Quartz, 1),
            (high, Pebble::Slate, 3),
            (high + 1, Pebble::Slate, 4),
        ] {
            assert_eq!(
                host.transaction(|host, tx| Ok::<_, ()>(
                    source.insert_at(host, tx, index, &resource, amount)
                )),
                Ok(Ok(amount))
            );
        }
        let combined = CombinedResourceHandler::new(
            &mut host,
            vec![
                Box::new(resource_util::VoidingResourceHandler),
                Box::new(source),
            ],
        );
        let from = Capability::<dyn ResourceHandler<Pebble>> {
            handler: Box::new(
                RangedResourceHandler::of(&mut host, Box::new(combined), 1..usize::MAX).unwrap(),
            ),
            owner: Some("attached_storage_test"),
        };
        let to = &destination as &dyn ResourceHandler<Pebble>;
        assert_eq!(
            move_first(&mut host, &from, to, |_| true, 4, None),
            Ok(Some((Pebble::Quartz, 1)))
        );
        assert_eq!(
            move_resources(&mut host, &from, to, |_| true, 4, None),
            Ok(3)
        );
        assert_eq!(
            move_first(&mut host, &from, to, |_| true, 1, None),
            Ok(None)
        );
        assert_eq!(
            move_resources(&mut host, &from, to, |_| true, 1, None),
            Ok(0)
        );
        assert_eq!(destination.resource(&mut host, 0), Some(Pebble::Quartz));
        assert_eq!(destination.amount(&mut host, 1), 3);
        assert_eq!(
            memory.get_existing(&mut host as &mut dyn ServerHost, CHUNK, |value| value
                .cloned()),
            Ok(Some(StackList::from([(high + 1, (Pebble::Slate, 4))])))
        );
    });
}

/// Handlers built with keys from a prior installation refuse reads and writes.
#[test]
fn stale_keys_refuse_attached_storage_reads_and_writes() {
    in_world(|| {
        let items = install_items();
        resolve(&["minecraft:dirt"], &[]);
        let mut host = FakeHost::new();
        let dirt = item("minecraft:dirt");
        let slots = AttachedItems::new(&items, CHUNK, 4, 64, |_| true);
        assert_eq!(
            host.transaction(|host, tx| slots.insert_at(host, tx, 0, &dirt, 10)),
            Ok(10)
        );
        assert_eq!(slots.amount(&mut host, 0), 10);

        let fresh_items = install_items();
        resolve(&["minecraft:dirt"], &[]);
        let dirt = item("minecraft:dirt");
        assert_eq!(slots.amount(&mut host, 0), 0);
        assert_eq!(slots.resource(&mut host, 0), None);
        assert_eq!(
            host.transaction(|host, tx| slots.insert_at(host, tx, 0, &dirt, 5)),
            Ok(0)
        );
        assert_eq!(
            host.transaction(|host, tx| slots.extract_at(host, tx, 0, &dirt, 5)),
            Ok(0)
        );
        let fresh_slots = AttachedItems::new(&fresh_items, CHUNK, 4, 64, |_| true);
        assert_eq!(fresh_slots.amount(&mut host, 0), 10);

        let energy = install_energy();
        let mut host = FakeHost::new();
        let battery = AttachedEnergy::new(&energy, CHUNK, 100, 100, 100);
        assert_eq!(
            host.transaction(|host, tx| battery.insert(host, tx, 40)),
            Ok(40)
        );
        assert_eq!(battery.amount(&mut host), 40);

        let fresh_energy = install_energy();
        assert_eq!(battery.amount(&mut host), 0);
        assert_eq!(
            host.transaction(|host, tx| battery.insert(host, tx, 10)),
            Ok(0)
        );
        assert_eq!(
            host.transaction(|host, tx| battery.extract(host, tx, 10)),
            Ok(0)
        );
        let fresh_battery = AttachedEnergy::new(&fresh_energy, CHUNK, 100, 100, 100);
        assert_eq!(fresh_battery.amount(&mut host), 40);
    });
}
