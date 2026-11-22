use super::*;
use crate::testing::FakeHost;
use crate::{Capability, ItemResource, Tag};
use std::{cell::RefCell, rc::Rc};

/// A mod's own resource, which the game knows nothing of.
#[derive(Clone, Debug, PartialEq)]
struct Chemical(&'static str);

/// Tanks of up to 100 of a chemical each, shared with the test to read back.
#[derive(Clone)]
struct Tanks(Rc<RefCell<Vec<Option<(Chemical, u32)>>>>);

impl Tanks {
    fn new(held: Vec<Option<(Chemical, u32)>>) -> Self {
        Self(Rc::new(RefCell::new(held)))
    }
    fn held(&self) -> Vec<Option<(Chemical, u32)>> {
        self.0.borrow().clone()
    }
}

impl ResourceHandler<Chemical> for Tanks {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.0.borrow().len()
    }
    fn resource(&self, _: &mut dyn ServerHost, index: usize) -> Option<Chemical> {
        self.0.borrow().get(index)?.as_ref().map(|(c, _)| c.clone())
    }
    fn amount(&self, _: &mut dyn ServerHost, index: usize) -> u32 {
        self.0
            .borrow()
            .get(index)
            .cloned()
            .flatten()
            .map_or(0, |(_, a)| a)
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&Chemical>) -> u32 {
        100
    }
    fn insert_at(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        index: usize,
        chemical: &Chemical,
        amount: u32,
    ) -> u32 {
        let mut tanks = self.0.borrow_mut();
        let Some(tank) = tanks.get_mut(index) else {
            return 0;
        };
        let held = match tank {
            Some((other, _)) if other != chemical => return 0,
            Some((_, held)) => *held,
            None => 0,
        };
        let went = amount.min(100 - held);
        if went > 0 {
            *tank = Some((chemical.clone(), held + went));
        }
        went
    }
    fn extract_at(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        index: usize,
        chemical: &Chemical,
        amount: u32,
    ) -> u32 {
        let mut tanks = self.0.borrow_mut();
        let Some(tank) = tanks.get_mut(index) else {
            return 0;
        };
        let Some((held_chemical, held)) = tank.clone().filter(|(c, _)| c == chemical) else {
            return 0;
        };
        let came = amount.min(held);
        *tank = (held > came).then(|| (held_chemical, held - came));
        came
    }
}

const HYDROGEN: Chemical = Chemical("test:hydrogen");

#[test]
fn a_combined_handler_numbers_both_and_fills_the_second_when_the_first_is_full() {
    let full = Some((HYDROGEN, 100));
    let first = Tanks::new(vec![full.clone(), full.clone()]);
    let second = Tanks::new(vec![None, None]);
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let combined = CombinedResourceHandler::new(
        host,
        vec![
            Box::new(first.clone()) as Box<dyn ResourceHandler<Chemical>>,
            Box::new(second.clone()),
        ],
    );
    assert_eq!(combined.size(host), 4);
    assert_eq!(combined.amount(host, 1), 100);
    assert_eq!(combined.resource(host, 2), None);
    let went = host.transaction(|host, tx| combined.insert(host, tx, &HYDROGEN, 150));
    assert_eq!(went, Ok(150));
    assert_eq!(first.held(), vec![full.clone(), full.clone()]);
    assert_eq!(second.held(), vec![full, Some((HYDROGEN, 50))]);
}

#[test]
fn combined_indices_do_not_shift_when_a_child_changes_size() {
    let host = &mut FakeHost::new();
    let first = Tanks::new(vec![None]);
    let second = Tanks::new(vec![Some((HYDROGEN, 23))]);
    let combined =
        CombinedResourceHandler::new(host, vec![Box::new(first.clone()), Box::new(second)]);
    first.0.borrow_mut().push(Some((HYDROGEN, 9)));
    assert_eq!(combined.size(host), 2);
    assert_eq!(combined.amount(host, 1), 23);
}

#[test]
fn ranged_checks_constructor_bounds_and_maps_the_single_index() {
    let host = &mut FakeHost::new();
    let tanks = Tanks::new(vec![Some((HYDROGEN, 10)), Some((HYDROGEN, 21))]);
    assert!(RangedResourceHandler::of(host, Box::new(tanks.clone()), 1..3).is_none());
    assert!(RangedResourceHandler::of(host, Box::new(tanks.clone()), 1..1).is_none());
    assert!(
        RangedResourceHandler::of_single_index(host, Box::new(tanks.clone()), usize::MAX).is_none()
    );
    let ranged = RangedResourceHandler::of_single_index(host, Box::new(tanks), 1).unwrap();
    assert_eq!(ranged.size(host), 1);
    assert_eq!(ranged.amount(host, 0), 21);
    assert_eq!(ranged.amount(host, 1), 0);
}

#[test]
fn an_empty_index_is_full_only_when_it_cannot_hold_anything() {
    let (mut fake, r) = items_host(&["minecraft:stone"]);
    chest(&mut fake, A, &[Some((&r[0], 64))]);
    chest(&mut fake, B, &[None]);
    let occupied = items_at(&mut fake, A);
    let empty = items_at(&mut fake, B);
    let host: &mut dyn ServerHost = &mut fake;
    let combined = CombinedResourceHandler::new(host, vec![Box::new(occupied), Box::new(empty)]);
    assert!(!is_full(host, &combined));

    fake.level.block_entity(B).items.as_mut().unwrap().limit = 0;
    assert!(is_full(&mut fake, &combined));
}

#[test]
fn insert_stacking_tops_up_a_holding_index_before_an_empty_one() {
    let tanks = Tanks::new(vec![None, Some((HYDROGEN, 10))]);
    let handler: &dyn ResourceHandler<Chemical> = &tanks;
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let went = insert_stacking(host, handler, &HYDROGEN, 95, None);
    assert_eq!(went, Ok(95));
    // The default `insert` would have filled index 0 first.
    assert_eq!(
        tanks.held(),
        vec![Some((HYDROGEN, 5)), Some((HYDROGEN, 100))]
    );
}

#[test]
fn the_comparator_reads_0_when_empty_15_when_full_and_vanillas_steps_between() {
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let signal = |host: &mut dyn ServerHost, held| comparator_signal(host, &Tanks::new(held));
    assert_eq!(signal(&mut *host, vec![]), 0);
    assert_eq!(signal(&mut *host, vec![None, None]), 0);
    assert_eq!(
        signal(
            &mut *host,
            vec![Some((HYDROGEN, 100)), Some((HYDROGEN, 100))]
        ),
        15
    );
    // One unit in one of two tanks: any content gives at least 1.
    assert_eq!(signal(&mut *host, vec![Some((HYDROGEN, 1)), None]), 1);
    // Half full: floor(0.5 * 14) + 1.
    assert_eq!(signal(&mut *host, vec![Some((HYDROGEN, 100)), None]), 8);
    // An overfilled slot counts as full, not as filling the other slot.
    assert_eq!(
        signal(host, vec![Some((HYDROGEN, u32::MAX)), Some((HYDROGEN, 50))]),
        11
    );
}

#[test]
fn the_comparator_counts_an_occupied_zero_capacity_index_as_zero_fill() {
    let (mut fake, r) = items_host(&["minecraft:stone"]);
    chest(&mut fake, A, &[Some((&r[0], 64))]);
    chest(&mut fake, B, &[Some((&r[0], 1))]);
    fake.level.block_entity(B).items.as_mut().unwrap().limit = 0;
    let full = items_at(&mut fake, A);
    let zero_capacity = items_at(&mut fake, B);
    let host: &mut dyn ServerHost = &mut fake;
    let tanks = CombinedResourceHandler::new(host, vec![Box::new(full), Box::new(zero_capacity)]);
    assert_eq!(comparator_signal(host, &tanks), 8);
}

/// A handler of `size` indices each holding `u32::MAX` of hydrogen in a capacity of 1: amounts
/// and sizes past anything sane, which no query may panic on.
struct Oversized(usize);

impl ResourceHandler<Chemical> for Oversized {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.0
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<Chemical> {
        Some(HYDROGEN)
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        u32::MAX
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&Chemical>) -> u32 {
        1
    }
    fn insert_at(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        _: usize,
        _: &Chemical,
        _: u32,
    ) -> u32 {
        0
    }
    fn extract_at(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        _: usize,
        _: &Chemical,
        _: u32,
    ) -> u32 {
        0
    }
}

#[test]
fn oversized_amounts_and_sizes_cap_instead_of_overflowing() {
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let full = &Oversized(1) as &dyn ResourceHandler<Chemical>;
    assert_eq!(comparator_signal(host, full), 15);
    let combined = CombinedResourceHandler::new(
        host,
        vec![
            Box::new(Oversized(usize::MAX)) as Box<dyn ResourceHandler<Chemical>>,
            Box::new(Oversized(2)),
        ],
    );
    assert_eq!(combined.size(host), usize::MAX);
}

/// One index that holds nothing yet answers one more than it was asked: a mod bug.
struct Liar;

impl<R> ResourceHandler<R> for Liar {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<R> {
        None
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&R>) -> u32 {
        u32::MAX
    }
    fn insert_at(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        _: usize,
        _: &R,
        amount: u32,
    ) -> u32 {
        amount.saturating_add(1)
    }
    fn extract_at(&self, _: &mut dyn ServerHost, _: &Transaction, _: usize, _: &R, _: u32) -> u32 {
        0
    }
}

/// `handler` as a capability `owner`'s provider answered with.
fn capability<R: 'static>(
    handler: impl ResourceHandler<R> + 'static,
    owner: &'static str,
) -> Capability<dyn ResourceHandler<R>> {
    Capability {
        handler: Box::new(handler),
        owner: Some(owner),
    }
}

#[test]
fn a_part_answering_more_than_asked_shows_through_every_total() {
    let mut fake = FakeHost::new();
    let host: &mut dyn ServerHost = &mut fake;
    let tank_then_liar = |host: &mut dyn ServerHost| {
        CombinedResourceHandler::new(
            host,
            vec![
                Box::new(Tanks::new(vec![None])) as Box<dyn ResourceHandler<Chemical>>,
                Box::new(Liar),
            ],
        )
    };
    let insert = |host: &mut dyn ServerHost, handler: &dyn ResourceHandler<Chemical>, amount| {
        host.transaction(|host, tx| handler.insert(host, tx, &HYDROGEN, amount))
    };
    // In either case the first tank takes 100, and the liar exceeds the remainder.
    for amount in [150, u32::MAX] {
        let inner = tank_then_liar(host);
        assert_eq!(
            insert(host, &inner, amount),
            Err(Overanswered { owner: None })
        );
    }
    // The ranged wrapper inherits the default slot-less implementation.
    let indices = CombinedResourceHandler::new(
        host,
        vec![
            Box::new(Tanks::new(vec![Some((HYDROGEN, 90))])) as Box<dyn ResourceHandler<Chemical>>,
            Box::new(Liar),
        ],
    );
    let indices = RangedResourceHandler::of(host, Box::new(indices), 0..2).unwrap();
    assert_eq!(
        insert(host, &indices, 20),
        Err(Overanswered { owner: None })
    );
    assert_eq!(
        insert_stacking(host, &indices, &HYDROGEN, 20, None),
        Err(Overanswered { owner: None })
    );
}

/// A host with `ids` registered, stacking to 64, and their resources.
fn items_host(names: &[&str]) -> (FakeHost, Vec<ItemResource>) {
    use crate::prelude::ItemsHost;
    let mut host = FakeHost::new();
    let ids: Vec<_> = names
        .iter()
        .map(|id| crate::Identifier::parse(id).unwrap())
        .collect();
    host.registries.resolve_items(ids.clone()).unwrap();
    for name in names {
        host.registries
            .item_component(name, "minecraft:max_stack_size", Tag::Int(64));
    }
    let resources = ids
        .iter()
        .map(|id| ItemResource::of(&crate::ItemStack::new(host.item_key(id).unwrap(), 1).unwrap()))
        .collect();
    (host, resources)
}

/// A chest at `pos` holding `slots`, in the journaled level, so an abort puts it back.
fn chest(host: &mut FakeHost, pos: [i32; 3], slots: &[Option<(&ItemResource, u32)>]) {
    let entity = host.level.block_entity(pos).with_items(slots.len());
    for (slot, held) in slots.iter().enumerate() {
        entity.items.as_mut().unwrap().slots[slot] = held.map(|(r, n)| r.to_stack(n).unwrap());
    }
}

fn items_at(host: &mut FakeHost, pos: [i32; 3]) -> Capability<dyn ResourceHandler<ItemResource>> {
    let level = host.level.level();
    let server: &mut dyn ServerHost = host;
    level.capability(server, &crate::ITEMS, pos, None).unwrap()
}

/// How many each slot of the chest at `pos` holds.
fn counts(host: &FakeHost, pos: [i32; 3]) -> Vec<u32> {
    let entity = host.level.block_entity_at(pos).unwrap();
    let slots = &entity.items.as_ref().unwrap().slots;
    slots
        .iter()
        .map(|s| s.as_ref().map_or(0, |s| s.count()))
        .collect()
}

const A: [i32; 3] = [0, 64, 0];
const B: [i32; 3] = [1, 64, 0];

#[test]
fn move_first_moves_one_resource_and_an_abort_puts_it_back() {
    let (mut fake, r) = items_host(&["minecraft:stone", "minecraft:dirt"]);
    let (stone, dirt) = (&r[0], &r[1]);
    chest(
        &mut fake,
        A,
        &[Some((stone, 5)), Some((dirt, 3)), Some((stone, 7))],
    );
    chest(&mut fake, B, &[None, Some((stone, 10))]);
    let (from, to) = (items_at(&mut fake, A), items_at(&mut fake, B));

    // Aborted: both chests as they were.
    let host: &mut dyn ServerHost = &mut fake;
    let aborted: Result<(), _> = host.transaction(|host, tx| {
        Err(move_first_stacking(
            host,
            &from,
            &to,
            |_| true,
            64,
            Some(tx),
        ))
    });
    assert_eq!(aborted, Err(Ok(Some((stone.clone(), 12)))));
    assert_eq!(counts(&fake, A), [5, 3, 7]);
    assert_eq!(counts(&fake, B), [0, 10]);

    // Committed, stacking: onto the 10 stone, not into the empty slot.
    let host: &mut dyn ServerHost = &mut fake;
    let moved = move_first_stacking(host, &from, &to, |_| true, 64, None);
    assert_eq!(moved, Ok(Some((stone.clone(), 12))));
    assert_eq!(counts(&fake, A), [0, 3, 0]);
    assert_eq!(counts(&fake, B), [0, 22]);

    // `move_first` moves the first resource that goes, the dirt, and only it.
    let host: &mut dyn ServerHost = &mut fake;
    let moved = host.transaction(|host, tx| move_first(host, &from, &to, |_| true, 2, Some(tx)));
    assert_eq!(moved, Ok(Some((dirt.clone(), 2))));
    assert_eq!(counts(&fake, A), [0, 1, 0]);
    assert_eq!(counts(&fake, B), [2, 22]);
}

#[test]
fn find_extractable_probes_the_whole_int_amount_and_rolls_back() {
    struct WholeSlot(Capability<dyn ResourceHandler<ItemResource>>);

    impl ResourceHandler<ItemResource> for WholeSlot {
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
            item: Option<&ItemResource>,
        ) -> u32 {
            self.0.capacity(host, index, item)
        }
        fn insert_at(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            item: &ItemResource,
            amount: u32,
        ) -> u32 {
            self.0.insert_at(host, tx, index, item, amount)
        }
        fn extract_at(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            item: &ItemResource,
            amount: u32,
        ) -> u32 {
            if amount != self.0.amount(host, index) {
                return 0;
            }
            self.0.extract_at(host, tx, index, item, amount)
        }
    }

    let (mut fake, r) = items_host(&["minecraft:stone", "minecraft:dirt"]);
    let (stone, dirt) = (&r[0], &r[1]);
    chest(&mut fake, A, &[Some((dirt, 3)), Some((stone, 5))]);
    let items = WholeSlot(items_at(&mut fake, A));
    let host: &mut dyn ServerHost = &mut fake;
    let found = host.transaction(|host, tx| {
        Ok::<_, ()>(find_extractable_resource(
            host,
            &items,
            |r| r == stone,
            Some(tx),
        ))
    });
    assert_eq!(found, Ok(Some(stone.clone())));
    assert_eq!(counts(&fake, A), [3, 5]);
    assert_eq!(fake.journal.closed(), [(2, false), (1, true)]);
}

#[test]
fn stacking_moves_each_allowed_resource_into_an_existing_stack() {
    let (mut fake, r) = items_host(&["minecraft:stone", "minecraft:dirt"]);
    let (stone, dirt) = (&r[0], &r[1]);
    chest(&mut fake, A, &[Some((stone, 5)), Some((dirt, 3))]);
    chest(&mut fake, B, &[None, Some((stone, 4)), Some((dirt, 6))]);
    let (from, to) = (items_at(&mut fake, A), items_at(&mut fake, B));
    assert_eq!(
        move_resources_stacking(&mut fake, &from, &to, |_| true, 8, None),
        Ok(8)
    );
    assert_eq!(counts(&fake, A), [0, 0]);
    assert_eq!(counts(&fake, B), [0, 9, 9]);
}

#[test]
fn each_extractability_probe_starts_with_unchanged_storage() {
    struct Probe(Capability<dyn ResourceHandler<ItemResource>>);

    impl ResourceHandler<ItemResource> for Probe {
        fn size(&self, host: &mut dyn ServerHost) -> usize {
            self.0.size(host)
        }
        fn resource(&self, host: &mut dyn ServerHost, slot: usize) -> Option<ItemResource> {
            self.0.resource(host, slot)
        }
        fn amount(&self, host: &mut dyn ServerHost, slot: usize) -> u32 {
            self.0.amount(host, slot)
        }
        fn capacity(
            &self,
            host: &mut dyn ServerHost,
            slot: usize,
            r: Option<&ItemResource>,
        ) -> u32 {
            self.0.capacity(host, slot, r)
        }
        fn insert_at(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
        fn extract_at(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            slot: usize,
            r: &ItemResource,
            amount: u32,
        ) -> u32 {
            if slot == 0 {
                assert_eq!(self.0.extract_at(host, tx, slot, r, amount), 3);
                return 0;
            }
            if self.0.amount(host, 0) == 3 {
                self.0.extract_at(host, tx, slot, r, amount)
            } else {
                0
            }
        }
    }

    let (mut fake, r) = items_host(&["minecraft:stone", "minecraft:dirt"]);
    chest(&mut fake, A, &[Some((&r[0], 3)), Some((&r[1], 5))]);
    let probe = Probe(items_at(&mut fake, A));
    assert_eq!(
        find_extractable_resource(&mut fake, &probe, |_| true, None),
        Some(r[1].clone())
    );
    assert_eq!(counts(&fake, A), [3, 5]);
}

#[test]
fn extract_first_only_takes_the_selected_resource_and_commits_its_root() {
    let (mut fake, r) = items_host(&["minecraft:stone", "minecraft:dirt"]);
    let (stone, dirt) = (&r[0], &r[1]);
    chest(
        &mut fake,
        A,
        &[Some((dirt, 3)), Some((stone, 5)), Some((stone, 7))],
    );
    let items = items_at(&mut fake, A);
    let taken = extract_first(&mut fake, &*items, |r| r == stone, 8, None);
    assert_eq!(taken, Some((stone.clone(), 8)));
    assert_eq!(counts(&fake, A), [3, 0, 4]);
    assert_eq!(fake.journal.closed().last(), Some(&(1, true)));
}

#[test]
fn optional_tokens_must_name_the_current_transaction() {
    let (mut fake, r) = items_host(&["minecraft:stone"]);
    let stone = &r[0];
    chest(&mut fake, A, &[Some((stone, 5))]);
    let items = items_at(&mut fake, A);
    let assert_refused = |host: &mut dyn ServerHost, token: Option<&Transaction>| {
        assert_eq!(insert_stacking(host, &items, stone, 1, token), Ok(0));
        assert_eq!(extract_first(host, &items, |_| true, 1, token), None);
        assert_eq!(
            find_extractable_resource(host, &items, |_| true, token),
            None
        );
    };
    let host: &mut dyn ServerHost = &mut fake;
    host.transaction(|host, parent| {
        assert_refused(host, None);
        host.transaction(|host, _| {
            assert_refused(host, Some(parent));
            Ok::<_, ()>(())
        })
    })
    .unwrap();
    assert_eq!(counts(&fake, A), [5]);
}

#[test]
fn an_overanswer_after_an_honest_part_undoes_the_whole_move() {
    let (mut fake, r) = items_host(&["minecraft:stone"]);
    let stone = &r[0];
    chest(&mut fake, A, &[Some((stone, 5))]);
    chest(&mut fake, B, &[Some((stone, 62))]);
    let from = items_at(&mut fake, A);
    let honest = items_at(&mut fake, B).handler;
    let host: &mut dyn ServerHost = &mut fake;
    let to = capability(
        CombinedResourceHandler::new(host, vec![honest, Box::new(Liar)]),
        "test:liar",
    );
    // B takes 2; the liar, asked for 3, answers 4. Both slot-less and indexed stacking
    // insertion identify the destination provider and restore the honest index.
    for stacking in [false, true] {
        let host: &mut dyn ServerHost = &mut fake;
        let moved = host.transaction(|host, tx| {
            if stacking {
                move_resources_stacking(host, &from, &to, |_| true, 5, Some(tx))
            } else {
                move_resources(host, &from, &to, |_| true, 5, Some(tx))
            }
        });
        assert_eq!(
            moved,
            Err(Overanswered {
                owner: Some("test:liar".into())
            })
        );
        assert_eq!(counts(&fake, A), [5]);
        assert_eq!(counts(&fake, B), [62]);
    }
    let host: &mut dyn ServerHost = &mut fake;
    assert_eq!(
        move_resources_stacking(host, &from, &*to, |_| true, 5, None),
        Err(Overanswered { owner: None })
    );
    assert_eq!(counts(&fake, A), [5]);
    assert_eq!(counts(&fake, B), [62]);
}

#[test]
fn a_caught_bulk_error_does_not_commit_earlier_moves_and_the_sink_sees_the_source() {
    struct ObservingSink {
        source: Capability<dyn ResourceHandler<ItemResource>>,
        sink: Capability<dyn ResourceHandler<ItemResource>>,
    }

    impl ResourceHandler<ItemResource> for ObservingSink {
        fn size(&self, host: &mut dyn ServerHost) -> usize {
            self.sink.size(host)
        }
        fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
            self.sink.resource(host, index)
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            self.sink.amount(host, index)
        }
        fn capacity(
            &self,
            host: &mut dyn ServerHost,
            index: usize,
            item: Option<&ItemResource>,
        ) -> u32 {
            self.sink.capacity(host, index, item)
        }
        fn insert_at(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            item: &ItemResource,
            amount: u32,
        ) -> u32 {
            if self.source.resource(host, 1).as_ref() == Some(item) {
                // The previous resource really moved before the second one's bad answer.
                assert_eq!(self.source.amount(host, 0), 0);
                assert_eq!(self.sink.amount(host, 0), 5);
                return amount + 1;
            }
            // The destination is called after the probe rolls back, before extraction.
            assert_eq!(self.source.amount(host, 0), 5);
            self.sink.insert_at(host, tx, index, item, amount)
        }
        fn extract_at(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    let (mut fake, r) = items_host(&["minecraft:stone", "minecraft:dirt"]);
    let (stone, dirt) = (&r[0], &r[1]);
    chest(&mut fake, A, &[Some((stone, 5)), Some((dirt, 3))]);
    chest(&mut fake, B, &[None, None]);
    let other_pos = [2, 64, 0];
    chest(&mut fake, other_pos, &[None]);
    let other = items_at(&mut fake, other_pos);
    let from = items_at(&mut fake, A);
    let to = capability(
        ObservingSink {
            source: items_at(&mut fake, A),
            sink: items_at(&mut fake, B),
        },
        "test:sink",
    );
    let host: &mut dyn ServerHost = &mut fake;
    host.transaction(|host, parent| {
        // Neither `None` under a parent nor a token naming an ancestor admits a move.
        assert_eq!(move_resources(host, &from, &to, |_| true, 8, None), Ok(0));
        host.transaction(|host, _| {
            assert_eq!(
                move_resources(host, &from, &to, |_| true, 8, Some(parent)),
                Ok(0)
            );
            Ok::<_, ()>(())
        })?;
        // Commit this unrelated edit even after catching the helper's failure.
        assert_eq!(other.insert_at(host, parent, 0, stone, 1), 1);
        assert_eq!(
            move_resources(host, &from, &to, |_| true, 8, Some(parent)),
            Err(Overanswered {
                owner: Some("test:sink".into())
            })
        );
        Ok::<_, ()>(())
    })
    .unwrap();
    assert_eq!(counts(&fake, A), [5, 3]);
    assert_eq!(counts(&fake, B), [0, 0]);
    assert_eq!(counts(&fake, other_pos), [1]);
}

#[test]
fn a_maximum_transfer_propagates_nested_overanswers_and_aborts_the_tank() {
    let (mut fake, r) = items_host(&["minecraft:stone"]);
    let stone = &r[0];
    chest(&mut fake, B, &[Some((stone, 62))]);
    let honest = items_at(&mut fake, B).handler;
    let host: &mut dyn ServerHost = &mut fake;
    let inner = CombinedResourceHandler::new(host, vec![honest, Box::new(Liar)]);
    let outer = CombinedResourceHandler::new(
        host,
        vec![
            Box::new(inner),
            Box::new(InfiniteResourceHandler::new(stone.clone())),
        ],
    );
    let from = capability(InfiniteResourceHandler::new(stone.clone()), "test:creative");
    let to = capability(outer, "test:liar");

    // A caught error cannot commit the honest tank's write or fall through to the last sink.
    let refused = host.transaction(|host, tx| Ok::<_, ()>(to.insert(host, tx, stone, u32::MAX)));
    assert_eq!(
        refused,
        Ok(Err(Overanswered {
            owner: Some("test:liar".into())
        }))
    );
    assert_eq!(counts(&fake, B), [62]);
    assert_eq!(fake.journal.closed().last(), Some(&(1, true)));

    let host: &mut dyn ServerHost = &mut fake;
    let moved = move_resources(host, &from, &to, |_| true, u32::MAX, None);
    assert_eq!(
        moved,
        Err(Overanswered {
            owner: Some("test:liar".into())
        })
    );
    assert_eq!(counts(&fake, B), [62]);
    assert_eq!(fake.journal.closed().last(), Some(&(1, false)));
    assert_eq!(fake.journal.depth(), 0);
}

#[test]
fn maximum_batch_moves_without_splitting_a_quantized_handler() {
    struct WholeBatch(bool);

    impl ResourceHandler<Chemical> for WholeBatch {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<Chemical> {
            self.0.then_some(HYDROGEN)
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            if self.0 { u32::MAX } else { 0 }
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&Chemical>) -> u32 {
            u32::MAX
        }
        fn insert_at(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &Chemical,
            amount: u32,
        ) -> u32 {
            if !self.0 && amount == u32::MAX {
                amount
            } else {
                0
            }
        }
        fn extract_at(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &Chemical,
            amount: u32,
        ) -> u32 {
            if self.0 && amount == u32::MAX {
                amount
            } else {
                0
            }
        }
    }

    let host: &mut dyn ServerHost = &mut FakeHost::new();
    let from = capability(WholeBatch(true), "test:source");
    let to = capability(
        CombinedResourceHandler::new(host, vec![Box::new(WholeBatch(false))]),
        "test:sink",
    );
    assert_eq!(
        move_resources(host, &from, &to, |_| true, u32::MAX, None),
        Ok(u32::MAX)
    );
    assert_eq!(
        move_resources_stacking(host, &from, &to, |_| true, u32::MAX, None),
        Ok(u32::MAX)
    );
    assert_eq!(
        move_first(host, &from, &to, |_| true, u32::MAX, None),
        Ok(Some((HYDROGEN, u32::MAX)))
    );
    assert_eq!(
        move_first_stacking(host, &from, &to, |_| true, u32::MAX, None),
        Ok(Some((HYDROGEN, u32::MAX)))
    );
}

#[test]
fn moves_accept_public_wrappers_without_registering_providers() {
    let host: &mut dyn ServerHost = &mut FakeHost::new();
    let source = RangedResourceHandler::of_single_index(
        host,
        Box::new(InfiniteResourceHandler::new(HYDROGEN)),
        0,
    )
    .unwrap();
    let sink = CombinedResourceHandler::new(host, vec![Box::new(VoidingResourceHandler)]);
    let (source, sink): (
        &dyn ResourceHandler<Chemical>,
        &dyn ResourceHandler<Chemical>,
    ) = (&source, &sink);
    assert_eq!(
        move_resources(host, source, sink, |_| true, 17, None),
        Ok(17)
    );
}
