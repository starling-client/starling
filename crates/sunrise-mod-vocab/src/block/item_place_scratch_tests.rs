use super::*;
use crate::prelude::ItemsHost;
use crate::testing::FakeHost;
use crate::{Host, Identifier, ScratchError, Tag, TransactionError};
use std::panic::{AssertUnwindSafe, catch_unwind};

const CHEST: [i32; 3] = [0, 64, 0];

/// Fresh fake host with `items` resolved and configured to their given max stack sizes.
fn host_with(items: &[(&str, i32)]) -> (FakeHost, Vec<ItemResource>) {
    crate::parts::install_input(crate::__private::parts_installation::InstalledInput::default());
    let mut host = FakeHost::new();
    let ids: Vec<_> = items
        .iter()
        .map(|(id, _)| Identifier::parse(id).unwrap())
        .collect();
    host.registries.resolve_items(ids.clone()).unwrap();
    for &(id, max_stack) in items {
        host.registries
            .item_component(id, "minecraft:max_stack_size", Tag::Int(max_stack));
    }
    let resources = ids
        .iter()
        .map(|id| ItemResource::of(&ItemStack::new(host.item_key(id).unwrap(), 1).unwrap()))
        .collect();
    (host, resources)
}

fn with_custom_value(resource: &ItemResource, value: i32) -> ItemResource {
    let mut stack = resource.to_stack(1).unwrap();
    let data = Tag::Compound([("value".into(), Tag::Int(value))].into_iter().collect());
    stack
        .set_components(
            [("minecraft:custom_data".into(), data)]
                .into_iter()
                .collect(),
        )
        .unwrap();
    ItemResource::of(&stack)
}

fn seed_chest(host: &mut FakeHost, stack: ItemStack) {
    host.level
        .block_entity(CHEST)
        .with_items(1)
        .items
        .as_mut()
        .unwrap()
        .slots[0] = Some(stack);
}

fn chest_count(host: &FakeHost) -> u32 {
    host.level
        .block_entity_at(CHEST)
        .and_then(|be| be.items.as_ref())
        .and_then(|items| items.slots[0].as_ref())
        .map_or(0, ItemStack::count)
}

struct NoTx;
impl Host for NoTx {}
crate::impl_host_areas!(NoTx);
impl ServerHost for NoTx {}
crate::impl_server_host_areas!(NoTx);

#[test]
fn scratch_stack_commits_same_item_ops_refuses_other_items_and_caps_at_max_stack() {
    let (mut host, r) = host_with(&[("minecraft:stone", 16), ("minecraft:dirt", 64)]);
    let (stone, dirt) = (&r[0], &r[1]);
    let stone_tagged = with_custom_value(stone, 7);

    let committed = host.scratch_stack(stone.to_stack(5).unwrap(), |host, tx, place| {
        assert_eq!(place.amount(host), 5);
        assert_eq!(place.resource(host), Some(stone.clone()));

        // Different item is refused for insert, extract, and exchange (leaving stack intact).
        assert_eq!(place.insert(host, tx, dirt, 4), Ok(0));
        assert_eq!(place.extract(host, tx, dirt, 1), Ok(0));
        assert_eq!(place.exchange(host, tx, dirt, 5), Ok(0));
        assert_eq!(place.amount(host), 5);
        assert_eq!(place.resource(host), Some(stone.clone()));

        // Same-item insert respects the item's max stack size.
        assert_eq!(place.insert(host, tx, stone, 20), Ok(11));
        assert_eq!(place.amount(host), 16);
        assert_eq!(place.insert(host, tx, stone, 1), Ok(0));

        // Different components cannot stack onto the occupied slot.
        assert_eq!(place.insert(host, tx, &stone_tagged, 1), Ok(0));

        // Partial extract reduces the count.
        assert_eq!(place.extract(host, tx, stone, 12), Ok(12));
        assert_eq!(place.amount(host), 4);

        // Same-item exchange with different components succeeds for the whole stack.
        assert_eq!(place.exchange(host, tx, &stone_tagged, 4), Ok(4));
        assert_eq!(place.amount(host), 4);
        assert_eq!(place.resource(host), Some(stone_tagged.clone()));

        // Whole-stack component edit updates components without changing item or count.
        assert_eq!(place.edit(host, tx, |editor| editor.set_damage(3)), Ok(()));
        assert_eq!(
            place.with_stack(host, |view| (view.count(), view.damage())),
            Some((4, 3))
        );
        let damaged = place.resource(host).unwrap();

        // Emptying the slot still keeps the fixed item (StackItemAccess): other items refuse.
        assert_eq!(place.extract(host, tx, &damaged, 4), Ok(4));
        assert_eq!(place.amount(host), 0);
        assert_eq!(place.with_stack(host, |_| ()), None);
        assert_eq!(
            place.edit(host, tx, |editor| editor.set_damage(1)),
            Err(ItemPlaceEditError::NotHeld)
        );
        assert_eq!(place.insert(host, tx, dirt, 1), Ok(0));

        // Reinserting the original item succeeds.
        assert_eq!(place.insert(host, tx, &stone_tagged, 3), Ok(3));
        assert_eq!(place.amount(host), 3);
        Ok::<_, ()>(42)
    });

    assert_eq!(committed, Ok((42, Some(stone_tagged.to_stack(3).unwrap()))));
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());

    // Committing an emptied scratch_stack publishes None.
    let emptied = host.scratch_stack(stone.to_stack(2).unwrap(), |host, tx, place| {
        assert_eq!(place.extract(host, tx, stone, 2), Ok(2));
        Ok::<_, ()>("empty")
    });
    assert_eq!(emptied, Ok(("empty", None)));
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());
}

#[test]
fn scratch_stack_abort_publishes_nothing_and_rolls_back_world() {
    let (mut host, r) = host_with(&[("minecraft:stone", 64)]);
    let stone = &r[0];
    seed_chest(&mut host, stone.to_stack(10).unwrap());
    let level = host.level.level();
    let chest = level.capability(&mut host, &ITEMS, CHEST, None).unwrap();

    let outcome = host.scratch_stack(stone.to_stack(5).unwrap(), |host, tx, place| {
        assert_eq!(chest.extract_at(host, tx, 0, stone, 4), Ok(4));
        assert_eq!(place.insert(host, tx, stone, 4), Ok(4));
        assert_eq!(place.amount(host), 9);
        assert_eq!(chest.amount(host, 0), 6);
        Err::<(), _>("aborted")
    });

    assert_eq!(outcome, Err(ScratchError::Aborted("aborted")));
    assert_eq!(chest_count(&host), 10);
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());
}

#[test]
fn scratch_slots_allows_item_change_replaces_slot_zero_and_overflows_to_slot_one() {
    let (mut host, r) = host_with(&[
        ("minecraft:bucket", 16),
        ("minecraft:water_bucket", 1),
        ("minecraft:lava_bucket", 1),
        ("minecraft:stone", 16),
    ]);
    let (bucket, water, lava, stone) = (&r[0], &r[1], &r[2], &r[3]);

    // Single stack: exchange replaces slot 0 in place and leaves slot 1 empty.
    let single = host.scratch_slots(bucket.to_stack(1).unwrap(), |host, tx, place| {
        assert_eq!(place.exchange(host, tx, water, 1), Ok(1));
        assert_eq!(place.resource(host), Some(water.clone()));
        assert_eq!(place.amount(host), 1);
        Ok::<_, ()>("single")
    });
    assert_eq!(
        single,
        Ok(("single", (Some(water.to_stack(1).unwrap()), None)))
    );

    // Multi-stack: OneByOne exchange keeps remainder in slot 0 and overflows changed item to slot 1.
    let multi = host.scratch_slots(bucket.to_stack(3).unwrap(), |host, tx, place| {
        let one = ItemPlace::OneByOne(Box::new(place.clone()));
        assert_eq!(one.amount(host), 1);
        assert_eq!(one.exchange(host, tx, water, 1), Ok(1));
        // Reads and extracts target slot 0 only.
        assert_eq!(place.amount(host), 2);
        assert_eq!(place.resource(host), Some(bucket.clone()));
        assert_eq!(place.extract(host, tx, water, 1), Ok(0));
        // Slot 1 is full (max stack 1), so another distinct item is refused.
        assert_eq!(place.insert(host, tx, lava, 1), Ok(0));
        Ok::<_, ()>("overflow")
    });
    assert_eq!(
        multi,
        Ok((
            "overflow",
            (
                Some(bucket.to_stack(2).unwrap()),
                Some(water.to_stack(1).unwrap()),
            )
        ))
    );

    // Sequential fill of the same item caps slot 0 at max_stack, then spills into slot 1.
    let sequential = host.scratch_slots(stone.to_stack(10).unwrap(), |host, tx, place| {
        assert_eq!(place.insert(host, tx, stone, 12), Ok(12));
        assert_eq!(place.amount(host), 16);
        assert_eq!(place.insert(host, tx, stone, 20), Ok(10));
        assert_eq!(place.extract(host, tx, stone, 16), Ok(16));
        // Slot 0 is now empty; reads and extracts do not fall through to slot 1.
        assert_eq!(place.amount(host), 0);
        assert_eq!(place.extract(host, tx, stone, 1), Ok(0));
        Ok::<_, ()>(())
    });
    assert_eq!(
        sequential,
        Ok(((), (None, Some(stone.to_stack(16).unwrap()))))
    );

    // Abort publishes neither slot.
    let aborted = host.scratch_slots(bucket.to_stack(2).unwrap(), |host, tx, place| {
        assert_eq!(place.extract(host, tx, bucket, 1), Ok(1));
        assert_eq!(place.insert(host, tx, water, 1), Ok(1));
        Err::<(), _>("nope")
    });
    assert_eq!(aborted, Err(ScratchError::Aborted("nope")));
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());
}

#[test]
fn restore_before_fallback_restores_lent_slots_after_nested_abort() {
    let (mut host, r) = host_with(&[
        ("minecraft:stone", 64),
        ("minecraft:bucket", 16),
        ("minecraft:water_bucket", 1),
        ("minecraft:lava_bucket", 1),
    ]);
    let (stone, bucket, water, lava) = (&r[0], &r[1], &r[2], &r[3]);

    // Single-slot restore before fallback (count and components).
    let initial_stone = with_custom_value(stone, 11).to_stack(5).unwrap();
    let expected_stone = with_custom_value(stone, 11).to_stack(4).unwrap();
    let single = host.scratch_stack(initial_stone.clone(), |host, tx, place| {
        let primary = host.transaction(|host, tx| {
            let current = place.resource(host).unwrap();
            assert_eq!(place.extract(host, tx, &current, 3), Ok(3));
            assert_eq!(place.edit(host, tx, |editor| editor.set_damage(4)), Ok(()));
            assert_eq!(
                place.with_stack(host, |view| (view.count(), view.damage())),
                Some((2, 4))
            );
            Err::<(), _>("primary failed")
        });
        assert_eq!(primary, Err(TransactionError::Aborted("primary failed")));

        // Fallback sees the exact restored stack before making its own edit.
        assert_eq!(
            place.with_stack(host, |view| view.project()).flatten(),
            Some(initial_stone.clone())
        );
        let restored = place.resource(host).unwrap();
        assert_eq!(place.extract(host, tx, &restored, 1), Ok(1));
        Ok::<_, ()>(())
    });
    assert_eq!(single, Ok(((), Some(expected_stone))));

    // Two-slot restore before fallback (both slot 0 and overflow slot 1 restored).
    let two_slot = host.scratch_slots(bucket.to_stack(3).unwrap(), |host, tx, place| {
        let primary = host.transaction(|host, tx| {
            assert_eq!(place.extract(host, tx, bucket, 1), Ok(1));
            assert_eq!(place.insert(host, tx, water, 1), Ok(1));
            assert_eq!(place.amount(host), 2);
            // Overflow slot is occupied by water_bucket.
            assert_eq!(place.insert(host, tx, lava, 1), Ok(0));
            Err::<(), _>("pickup failed")
        });
        assert_eq!(primary, Err(TransactionError::Aborted("pickup failed")));

        // Fallback sees slot 0 restored to 3 buckets and slot 1 restored to empty.
        assert_eq!(place.amount(host), 3);
        assert_eq!(place.resource(host), Some(bucket.clone()));
        assert_eq!(place.extract(host, tx, bucket, 1), Ok(1));
        assert_eq!(place.insert(host, tx, lava, 1), Ok(1));
        Ok::<_, ()>(())
    });
    assert_eq!(
        two_slot,
        Ok((
            (),
            (
                Some(bucket.to_stack(2).unwrap()),
                Some(lava.to_stack(1).unwrap()),
            )
        ))
    );
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());
}

#[test]
fn nested_success_remains_provisional_across_subsequent_abort_enclosing_abort_and_unwind() {
    let (mut host, r) = host_with(&[
        ("minecraft:stone", 64),
        ("minecraft:bucket", 16),
        ("minecraft:water_bucket", 1),
    ]);
    let (stone, bucket, water) = (&r[0], &r[1], &r[2]);
    seed_chest(&mut host, stone.to_stack(10).unwrap());
    let level = host.level.level();
    let chest = level.capability(&mut host, &ITEMS, CHEST, None).unwrap();

    // Nested commit is visible inside loan, later nested abort rolls back only the later write,
    // and enclosing loan abort discards everything without publishing.
    let aborted_stack = host.scratch_stack(stone.to_stack(5).unwrap(), |host, _tx, place| {
        host.transaction(|host, tx| {
            assert_eq!(place.extract(host, tx, stone, 2), Ok(2));
            assert_eq!(chest.extract_at(host, tx, 0, stone, 1), Ok(1));
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(place.amount(host), 3);
        assert_eq!(chest.amount(host, 0), 9);

        let second = host.transaction(|host, tx| {
            assert_eq!(place.extract(host, tx, stone, 2), Ok(2));
            assert_eq!(chest.extract_at(host, tx, 0, stone, 3), Ok(3));
            assert_eq!(place.amount(host), 1);
            Err::<(), _>("second")
        });
        assert_eq!(second, Err(TransactionError::Aborted("second")));
        assert_eq!(place.amount(host), 3);
        assert_eq!(chest.amount(host, 0), 9);

        Err::<(), _>("enclosing abort")
    });
    assert_eq!(aborted_stack, Err(ScratchError::Aborted("enclosing abort")));
    assert_eq!(chest_count(&host), 10);
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());

    // Same provisional behavior for two-slot scratch loans.
    let aborted_slots = host.scratch_slots(bucket.to_stack(3).unwrap(), |host, _tx, place| {
        host.transaction(|host, tx| {
            assert_eq!(place.extract(host, tx, bucket, 1), Ok(1));
            assert_eq!(place.insert(host, tx, water, 1), Ok(1));
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(place.amount(host), 2);

        let second = host.transaction(|host, tx| {
            assert_eq!(place.extract(host, tx, bucket, 2), Ok(2));
            assert_eq!(place.amount(host), 0);
            Err::<(), _>("second slots")
        });
        assert_eq!(second, Err(TransactionError::Aborted("second slots")));
        assert_eq!(place.amount(host), 2);

        Err::<(), _>("enclosing slots abort")
    });
    assert_eq!(
        aborted_slots,
        Err(ScratchError::Aborted("enclosing slots abort"))
    );
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());

    // Enclosing loan unwinding after a committed nested transaction discards both loan and world writes.
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let _ = host.scratch_stack(
            stone.to_stack(5).unwrap(),
            |host, _tx, place| -> Result<(), ()> {
                host.transaction(|host, tx| {
                    assert_eq!(place.extract(host, tx, stone, 3), Ok(3));
                    assert_eq!(chest.extract_at(host, tx, 0, stone, 4), Ok(4));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(place.amount(host), 2);
                panic!("enclosing unwind");
            },
        );
    }));
    assert!(caught.is_err());
    assert_eq!(chest_count(&host), 10);
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());
}

#[test]
fn committed_scratch_loans_stay_out_of_outer_transaction_journal() {
    let (mut host, r) = host_with(&[
        ("minecraft:stone", 64),
        ("minecraft:bucket", 16),
        ("minecraft:water_bucket", 1),
    ]);
    let (stone, bucket, water) = (&r[0], &r[1], &r[2]);

    host.with_test_transaction(|host, tx| {
        // Inner scratch_stack commits inside the still-open outer transaction.
        let mut escaped_stack = None;
        let ((), final_stack) = host
            .scratch_stack(stone.to_stack(5).unwrap(), |host, tx, place| {
                assert_eq!(place.extract(host, tx, stone, 2), Ok(2));
                escaped_stack = Some(place.clone());
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(final_stack, Some(stone.to_stack(3).unwrap()));

        // While the outer transaction is still open, the closed loan is dead and absent from its journal.
        let escaped_stack = escaped_stack.unwrap();
        let ItemPlace::Scratch(escaped_stack_slot) = &escaped_stack else {
            unreachable!("scratch_stack must lend ItemPlace::Scratch")
        };
        let escaped_stack_slot = *escaped_stack_slot;
        assert_eq!(host.scratch_live_count(), 0);
        assert_eq!(host.scratch_kept_frame_counts(), [0]);
        assert!(!host.scratch_kept_contains(0, escaped_stack_slot));
        assert_eq!(escaped_stack.with_stack(host, |_| ()), None);
        assert_eq!(escaped_stack.amount(host), 0);
        assert_eq!(escaped_stack.insert(host, tx, stone, 1), Ok(0));
        assert_eq!(escaped_stack.extract(host, tx, stone, 1), Ok(0));

        // Inner scratch_slots commits inside the same still-open outer transaction.
        let mut escaped_slots = None;
        let ((), final_slots) = host
            .scratch_slots(bucket.to_stack(2).unwrap(), |host, tx, place| {
                assert_eq!(place.extract(host, tx, bucket, 1), Ok(1));
                assert_eq!(place.insert(host, tx, water, 1), Ok(1));
                escaped_slots = Some(place.clone());
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(
            final_slots,
            (
                Some(bucket.to_stack(1).unwrap()),
                Some(water.to_stack(1).unwrap()),
            )
        );

        let escaped_slots = escaped_slots.unwrap();
        let ItemPlace::Scratch(escaped_slots_slot) = &escaped_slots else {
            unreachable!("scratch_slots must lend ItemPlace::Scratch")
        };
        let escaped_slots_slot = *escaped_slots_slot;
        assert_eq!(host.scratch_live_count(), 0);
        assert_eq!(host.scratch_kept_frame_counts(), [0]);
        assert!(!host.scratch_kept_contains(0, escaped_slots_slot));
        assert_eq!(escaped_slots.with_stack(host, |_| ()), None);
        assert_eq!(escaped_slots.amount(host), 0);
        assert_eq!(escaped_slots.insert(host, tx, bucket, 1), Ok(0));
        assert_eq!(escaped_slots.extract(host, tx, bucket, 1), Ok(0));

        (true, ())
    });

    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());
}

#[test]
fn nested_scratch_loan_leaves_outer_loan_journal_clean_and_outer_mutation_provisional() {
    let (mut host, r) = host_with(&[
        ("minecraft:stone", 64),
        ("minecraft:dirt", 64),
        ("minecraft:bucket", 16),
        ("minecraft:water_bucket", 1),
    ]);
    let (stone, dirt, bucket, water) = (&r[0], &r[1], &r[2], &r[3]);

    // White-box check: S2 inside S1 mutating both S1 and S2 removes S2 from S1's journal on commit
    // while keeping S1's snapshot in S1's journal and S1's mutation provisional.
    let (published, ()) =
        host.with_test_scratch_stack(stone.to_stack(5).unwrap(), |host, tx, s1| {
            let ItemPlace::Scratch(s1_slot) = s1 else {
                unreachable!("with_test_scratch_stack lends ItemPlace::Scratch")
            };
            let s1_slot = *s1_slot;
            let mut escaped_s2 = None;
            let ((), s2_final) = host
                .scratch_slots(bucket.to_stack(2).unwrap(), |host, tx, s2| {
                    assert_eq!(s1.extract(host, tx, stone, 2), Ok(2));
                    assert_eq!(s2.extract(host, tx, bucket, 1), Ok(1));
                    assert_eq!(s2.insert(host, tx, water, 1), Ok(1));
                    escaped_s2 = Some(s2.clone());
                    Ok::<_, ()>(())
                })
                .unwrap();
            assert_eq!(
                s2_final,
                (
                    Some(bucket.to_stack(1).unwrap()),
                    Some(water.to_stack(1).unwrap()),
                )
            );

            let escaped_s2 = escaped_s2.unwrap();
            let ItemPlace::Scratch(s2_slot) = &escaped_s2 else {
                unreachable!("scratch_slots must lend ItemPlace::Scratch")
            };
            let s2_slot = *s2_slot;
            assert_eq!(host.scratch_live_count(), 1);
            assert_eq!(host.scratch_kept_frame_counts(), [1]);
            assert!(!host.scratch_kept_contains(0, s2_slot));
            assert!(host.scratch_kept_contains(0, s1_slot));
            assert_eq!(s1.amount(host), 3);
            assert_eq!(escaped_s2.with_stack(host, |_| ()), None);
            assert_eq!(escaped_s2.amount(host), 0);
            assert_eq!(escaped_s2.insert(host, tx, bucket, 1), Ok(0));
            assert_eq!(escaped_s2.extract(host, tx, bucket, 1), Ok(0));

            // Abort S1 so S1's mutation is discarded and never published.
            (false, ())
        });
    assert_eq!(published, None);
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());

    // Public 3-level chain S0 -> S1 -> S2: S2 commits mutations to S0, S1, and S2;
    // aborting S1 rolls back S0's mutation that originated inside S2.
    let s0_result = host.scratch_stack(stone.to_stack(10).unwrap(), |host, _tx0, s0| {
        let s1_result = host.scratch_stack(dirt.to_stack(5).unwrap(), |host, tx1, s1| {
            let mut escaped_s2 = None;
            let ((), s2_final) = host
                .scratch_slots(bucket.to_stack(2).unwrap(), |host, tx2, s2| {
                    assert_eq!(s0.extract(host, tx2, stone, 3), Ok(3));
                    assert_eq!(s1.extract(host, tx2, dirt, 2), Ok(2));
                    assert_eq!(s2.extract(host, tx2, bucket, 1), Ok(1));
                    assert_eq!(s2.insert(host, tx2, water, 1), Ok(1));
                    escaped_s2 = Some(s2.clone());
                    Ok::<_, ()>(())
                })
                .unwrap();
            assert_eq!(
                s2_final,
                (
                    Some(bucket.to_stack(1).unwrap()),
                    Some(water.to_stack(1).unwrap()),
                )
            );
            assert_eq!(s0.amount(host), 7);
            assert_eq!(s1.amount(host), 3);
            let escaped_s2 = escaped_s2.unwrap();
            assert_eq!(escaped_s2.amount(host), 0);
            assert_eq!(escaped_s2.insert(host, tx1, bucket, 1), Ok(0));
            Err::<(), _>("abort s1")
        });
        assert_eq!(s1_result, Err(ScratchError::Aborted("abort s1")));
        assert_eq!(s0.amount(host), 10);
        Ok::<_, ()>(())
    });
    assert_eq!(s0_result, Ok(((), Some(stone.to_stack(10).unwrap()))));
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());
}

#[test]
fn foreign_host_scratch_place_reads_none_and_writes_zero() {
    let (mut host_a, r) = host_with(&[("minecraft:stone", 64)]);
    let stone = &r[0];
    let mut host_b = host_a.clone();

    let res_a = host_a.scratch_stack(stone.to_stack(5).unwrap(), |host_a, _tx_a, place_a| {
        let leaked = place_a.clone();
        let ItemPlace::Scratch(slot_a) = &leaked else {
            unreachable!("scratch_stack lends ItemPlace::Scratch")
        };
        let slot_a = *slot_a;

        // Cloned host B with no scratch journal tag yet rejects leaked place A.
        host_b
            .transaction(|host_b, tx_b| {
                assert_eq!(leaked.with_stack(host_b, |_| ()), None);
                assert_eq!(leaked.resource(host_b), None);
                assert_eq!(leaked.amount(host_b), 0);
                assert_eq!(leaked.insert(host_b, tx_b, stone, 1), Ok(0));
                assert_eq!(leaked.extract(host_b, tx_b, stone, 1), Ok(0));
                assert_eq!(
                    leaked.edit(host_b, tx_b, |editor| editor.set_damage(1)),
                    Err(ItemPlaceEditError::NotHeld)
                );
                Ok::<_, ()>(())
            })
            .unwrap();

        // Host B mints a distinct process-unique slot id and rejects leaked place A.
        let res_b = host_b.scratch_stack(stone.to_stack(8).unwrap(), |host_b, tx_b, place_b| {
            let ItemPlace::Scratch(slot_b) = place_b else {
                unreachable!("scratch_stack lends ItemPlace::Scratch")
            };
            let slot_b = *slot_b;
            assert_ne!(slot_a, slot_b);
            assert_ne!(slot_a.id(), slot_b.id());

            assert_eq!(leaked.with_stack(host_b, |_| ()), None);
            assert_eq!(leaked.amount(host_b), 0);
            assert_eq!(leaked.insert(host_b, tx_b, stone, 1), Ok(0));
            assert_eq!(leaked.extract(host_b, tx_b, stone, 1), Ok(0));
            assert_eq!(place_b.amount(host_b), 8);
            Ok::<_, ()>(())
        });
        assert_eq!(res_b, Ok(((), Some(stone.to_stack(8).unwrap()))));

        // Host A's own lent slot is untouched.
        assert_eq!(place_a.amount(host_a), 5);
        Ok::<_, ()>((leaked, slot_a))
    });

    let ((leaked, slot_a), published) = res_a.unwrap();
    assert_eq!(published, Some(stone.to_stack(5).unwrap()));
    assert_eq!(host_a.scratch_live_count(), 0);
    assert_eq!(host_b.scratch_live_count(), 0);

    // Dropping host A and constructing a successor host never aliases the earlier handle.
    drop(host_a);
    let (mut host_c, r_c) = host_with(&[("minecraft:stone", 64)]);
    let stone_c = &r_c[0];
    let res_c = host_c.scratch_stack(stone_c.to_stack(9).unwrap(), |host_c, tx_c, place_c| {
        let ItemPlace::Scratch(slot_c) = place_c else {
            unreachable!("scratch_stack lends ItemPlace::Scratch")
        };
        assert_ne!(slot_a, *slot_c);
        assert_ne!(slot_a.id(), slot_c.id());
        assert_eq!(leaked.with_stack(host_c, |_| ()), None);
        assert_eq!(leaked.amount(host_c), 0);
        assert_eq!(leaked.insert(host_c, tx_c, stone_c, 1), Ok(0));
        assert_eq!(leaked.extract(host_c, tx_c, stone_c, 1), Ok(0));
        assert_eq!(place_c.amount(host_c), 9);
        Ok::<_, ()>(())
    });
    assert_eq!(res_c, Ok(((), Some(stone_c.to_stack(9).unwrap()))));
    assert_eq!(host_c.scratch_live_count(), 0);
}

#[test]
fn wrong_transaction_context_is_refused_before_mutating_lent_scratch_slot() {
    let (mut host_a, r) = host_with(&[("minecraft:stone", 64)]);
    let stone = &r[0];
    let stone_tagged = with_custom_value(stone, 9);
    let mut host_b = host_a.clone();

    // Ancestor transaction context is refused before mutating.
    let ancestor = host_a.scratch_stack(stone.to_stack(5).unwrap(), |host, outer_tx, place| {
        host.transaction(|host, inner_tx| {
            assert_eq!((outer_tx.depth(), inner_tx.depth()), (0, 1));
            assert_eq!(
                place.insert(host, outer_tx, stone, 1),
                Err(TransactionError::InvalidContext)
            );
            assert_eq!(
                place.extract(host, outer_tx, stone, 1),
                Err(TransactionError::InvalidContext)
            );
            assert_eq!(
                place.exchange(host, outer_tx, &stone_tagged, 5),
                Err(TransactionError::InvalidContext)
            );
            assert_eq!(
                place.edit(host, outer_tx, |_| panic!(
                    "invalid context reached the editor"
                )),
                Err(ItemPlaceEditError::InvalidContext)
            );
            assert_eq!(place.amount(host), 5);
            assert_eq!(place.resource(host), Some(stone.clone()));
            assert_eq!(place.with_stack(host, |view| view.damage()), Some(0));

            // Inner context still works cleanly after the refused ancestor calls.
            assert_eq!(place.extract(host, inner_tx, stone, 1), Ok(1));
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(place.amount(host), 4);
        Ok::<_, ()>(())
    });
    assert_eq!(ancestor, Ok(((), Some(stone.to_stack(4).unwrap()))));

    // Foreign host transaction context at the same depth is refused before mutating.
    host_b
        .transaction(|_host_b, foreign_tx| {
            let res =
                host_a.scratch_stack(stone.to_stack(5).unwrap(), |host_a, own_tx, place_a| {
                    assert_eq!((foreign_tx.depth(), own_tx.depth()), (0, 0));
                    assert_eq!(
                        place_a.insert(host_a, foreign_tx, stone, 1),
                        Err(TransactionError::InvalidContext)
                    );
                    assert_eq!(
                        place_a.extract(host_a, foreign_tx, stone, 1),
                        Err(TransactionError::InvalidContext)
                    );
                    assert_eq!(
                        place_a.exchange(host_a, foreign_tx, &stone_tagged, 5),
                        Err(TransactionError::InvalidContext)
                    );
                    assert_eq!(
                        place_a.edit(host_a, foreign_tx, |_| panic!(
                            "invalid context reached the editor"
                        )),
                        Err(ItemPlaceEditError::InvalidContext)
                    );
                    assert_eq!(place_a.amount(host_a), 5);
                    assert_eq!(place_a.with_stack(host_a, |view| view.damage()), Some(0));
                    assert_eq!(place_a.extract(host_a, own_tx, stone, 2), Ok(2));
                    Ok::<_, ()>(())
                });
            assert_eq!(res, Ok(((), Some(stone.to_stack(3).unwrap()))));
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(host_a.scratch_live_count(), 0);
    assert_eq!(host_b.scratch_live_count(), 0);
}

#[test]
fn invalid_scratch_stack_and_unavailable_host_refuse_without_running_closure() {
    let (mut host, r) = host_with(&[
        ("minecraft:bucket", 16),
        ("minecraft:zero_max", 0),
        ("minecraft:over_max", 100),
    ]);
    let (bucket, zero_max, over_max) = (&r[0], &r[1], &r[2]);

    let unknown = ItemStack::new(ItemKey(u32::MAX), 1).unwrap();
    let cases = [
        (unknown, StackWriteError::UnknownItem),
        (
            zero_max.to_stack(1).unwrap(),
            StackWriteError::InvalidMaximum,
        ),
        (
            over_max.to_stack(1).unwrap(),
            StackWriteError::InvalidMaximum,
        ),
    ];

    for (invalid, expected) in &cases {
        let single = host.scratch_stack(invalid.clone(), |_, _, _| -> Result<(), ()> {
            panic!("closure must not run for invalid stack");
        });
        assert_eq!(single, Err(ScratchError::Stack(expected.clone())));
        let two = host.scratch_slots(invalid.clone(), |_, _, _| -> Result<(), ()> {
            panic!("closure must not run for invalid stack");
        });
        assert_eq!(two, Err(ScratchError::Stack(expected.clone())));
    }
    assert!(host.journal.closed().is_empty());
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());

    // `dispense_container` treats scratch validation failures as unhandled (`None`) rather
    // than falling through to `dispense_default`. Its one source-level transaction (which owns
    // the transfer, the slot result and the remainder) opens and rolls back with nothing in it.
    for (invalid, _) in &cases {
        let event = crate::DispenseItemEvent {
            dimension: "minecraft:overworld",
            pos: [0, 64, 0],
            facing: Direction::East,
            stack: invalid,
        };
        assert_eq!(
            crate::block::fluid_util::dispense_container(&mut host, &event),
            None
        );
    }
    assert_eq!(host.journal.closed().len(), cases.len());
    assert!(
        host.journal
            .closed()
            .iter()
            .all(|&(_, committed)| !committed)
    );
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());

    // Oversized initial stacks are admitted and preserved (matching `StackItemAccess` /
    // `ItemStacksResourceHandler`), while further inserts cap at the item's max stack size.
    let oversized = bucket.to_stack(20).unwrap();
    let single_over = host.scratch_stack(oversized.clone(), |host, tx, place| {
        assert_eq!(place.amount(host), 20);
        assert_eq!(place.insert(host, tx, bucket, 1), Ok(0));
        assert_eq!(place.extract(host, tx, bucket, 2), Ok(2));
        assert_eq!(place.amount(host), 18);
        assert_eq!(place.insert(host, tx, bucket, 1), Ok(0));
        Ok::<_, ()>(())
    });
    assert_eq!(single_over, Ok(((), Some(bucket.to_stack(18).unwrap()))));

    let two_over = host.scratch_slots(oversized, |host, tx, place| {
        assert_eq!(place.amount(host), 20);
        assert_eq!(place.extract(host, tx, bucket, 2), Ok(2));
        assert_eq!(place.amount(host), 18);
        // Slot 0 is still above max_stack (16), so an insert spills into slot 1.
        assert_eq!(place.insert(host, tx, bucket, 1), Ok(1));
        assert_eq!(place.amount(host), 18);
        Ok::<_, ()>(())
    });
    assert_eq!(
        two_over,
        Ok((
            (),
            (
                Some(bucket.to_stack(18).unwrap()),
                Some(bucket.to_stack(1).unwrap()),
            )
        ))
    );

    let valid = bucket.to_stack(1).unwrap();
    let mut no_tx = NoTx;
    assert_eq!(
        no_tx.scratch_stack(valid.clone(), |_, _, _| -> Result<(), ()> {
            panic!("closure must not run when transactions are unavailable");
        }),
        Err(ScratchError::Unavailable)
    );
    assert_eq!(
        no_tx.scratch_slots(valid, |_, _, _| -> Result<(), ()> {
            panic!("closure must not run when transactions are unavailable");
        }),
        Err(ScratchError::Unavailable)
    );
}

#[test]
fn panic_unwind_restores_world_and_ancestor_scratch_and_cleans_up_loan() {
    let (mut host, r) = host_with(&[
        ("minecraft:stone", 64),
        ("minecraft:dirt", 64),
        ("minecraft:bucket", 16),
        ("minecraft:water_bucket", 1),
    ]);
    let (stone, dirt, bucket, water) = (&r[0], &r[1], &r[2], &r[3]);
    seed_chest(&mut host, stone.to_stack(10).unwrap());

    let (published, ()) =
        host.with_test_scratch_stack(stone.to_stack(5).unwrap(), |host, tx, s1| {
            let level = host.level.level();
            let chest = level.capability(host, &ITEMS, CHEST, None).unwrap();

            // Nested scratch_stack panicking restores chest + ancestor S1 and cleans up owned S2.
            let mut leaked_stack = None;
            let caught_stack = catch_unwind(AssertUnwindSafe(|| {
                let _ = host.scratch_stack(
                    dirt.to_stack(4).unwrap(),
                    |host, tx, s2| -> Result<(), ()> {
                        assert_eq!(chest.extract_at(host, tx, 0, stone, 3), Ok(3));
                        assert_eq!(s1.extract(host, tx, stone, 2), Ok(2));
                        assert_eq!(s2.extract(host, tx, dirt, 1), Ok(1));
                        leaked_stack = Some(s2.clone());
                        panic!("nested scratch_stack panic");
                    },
                );
            }));
            assert!(caught_stack.is_err());
            assert_eq!(chest.amount(host, 0), 10);
            assert_eq!(s1.amount(host), 5);
            assert_eq!(host.scratch_live_count(), 1);
            assert_eq!(host.scratch_kept_frame_counts(), [0]);
            let leaked_stack = leaked_stack.unwrap();
            assert_eq!(leaked_stack.with_stack(host, |_| ()), None);
            assert_eq!(leaked_stack.amount(host), 0);
            assert_eq!(leaked_stack.insert(host, tx, dirt, 1), Ok(0));

            // Nested scratch_slots panicking restores chest + ancestor S1 and cleans up both slots.
            let mut leaked_slots = None;
            let caught_slots = catch_unwind(AssertUnwindSafe(|| {
                let _ = host.scratch_slots(
                    bucket.to_stack(2).unwrap(),
                    |host, tx, s2| -> Result<(), ()> {
                        assert_eq!(chest.extract_at(host, tx, 0, stone, 4), Ok(4));
                        assert_eq!(s1.extract(host, tx, stone, 4), Ok(4));
                        assert_eq!(s2.extract(host, tx, bucket, 1), Ok(1));
                        assert_eq!(s2.insert(host, tx, water, 1), Ok(1));
                        leaked_slots = Some(s2.clone());
                        panic!("nested scratch_slots panic");
                    },
                );
            }));
            assert!(caught_slots.is_err());
            assert_eq!(chest.amount(host, 0), 10);
            assert_eq!(s1.amount(host), 5);
            assert_eq!(host.scratch_live_count(), 1);
            assert_eq!(host.scratch_kept_frame_counts(), [0]);
            let leaked_slots = leaked_slots.unwrap();
            assert_eq!(leaked_slots.with_stack(host, |_| ()), None);
            assert_eq!(leaked_slots.amount(host), 0);
            assert_eq!(leaked_slots.insert(host, tx, bucket, 1), Ok(0));

            (true, ())
        });
    assert_eq!(published, Some(Some(stone.to_stack(5).unwrap())));
    assert_eq!(chest_count(&host), 10);
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());

    // Root scratch_stack panicking restores world state and leaves zero leaks.
    let level = host.level.level();
    let chest = level.capability(&mut host, &ITEMS, CHEST, None).unwrap();
    let caught_root = catch_unwind(AssertUnwindSafe(|| {
        let _ = host.scratch_stack(
            stone.to_stack(3).unwrap(),
            |host, tx, place| -> Result<(), ()> {
                assert_eq!(chest.extract_at(host, tx, 0, stone, 4), Ok(4));
                assert_eq!(place.extract(host, tx, stone, 1), Ok(1));
                panic!("root scratch_stack panic");
            },
        );
    }));
    assert!(caught_root.is_err());
    assert_eq!(chest_count(&host), 10);
    assert_eq!(host.journal.depth(), 0);
    assert_eq!(host.scratch_live_count(), 0);
    assert!(host.scratch_kept_frame_counts().is_empty());
}
