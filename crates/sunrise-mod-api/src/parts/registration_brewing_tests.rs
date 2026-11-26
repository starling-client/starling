#![cfg(test)]
// Mutable slot and cancellation contracts, separate from engine brewing rules.
use crate::*;
use std::sync::{Arc, Mutex};

struct NoHost;
impl Host for NoHost {}
crate::impl_host_areas!(NoHost);
impl ServerHost for NoHost {}
crate::impl_server_host_areas!(NoHost);

struct BrewModifier {
    cancel: bool,
    seen: Arc<Mutex<Vec<String>>>,
}

#[crate::__subscriptions]
impl Hooks for BrewModifier {
    fn potion_brew_pre(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PotionBrewPreEvent<'_, '_>,
    ) -> Flow {
        self.seen.lock().unwrap().push("pre".into());
        event
            .stacks
            .set(
                0,
                Some(crate::__private::part_tests::stack(
                    "minecraft:diamond",
                    1,
                    0,
                )),
            )
            .unwrap();
        let invalid = crate::__private::part_tests::stack("minecraft:diamond", 2, 0);
        assert_eq!(
            event.stacks.set(1, Some(invalid)),
            Err(StackWriteError::CountExceeded { maximum: 1 })
        );
        if self.cancel {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn potion_brew_post(&self, _: &mut dyn ServerHost, event: &mut PotionBrewPostEvent<'_, '_>) {
        let first = event
            .stacks
            .get(0)
            .flatten()
            .and_then(|stack| crate::__private::item_key_id(stack.item()))
            .unwrap();
        self.seen.lock().unwrap().push(format!("post {first}"));
        event
            .stacks
            .set(
                1,
                Some(crate::__private::part_tests::stack(
                    "minecraft:emerald",
                    1,
                    0,
                )),
            )
            .unwrap();
    }
}

#[test]
fn brewing_callbacks_keep_independent_slots_and_report_cancellation() {
    install_parts(&[], &crate::__private::part_tests::rule_schema(), |_| false).unwrap();
    for cancel in [true, false] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "brew-modifier",
            Box::new(BrewModifier {
                cancel,
                seen: seen.clone(),
            }),
        )]
        .into();
        testing::with_mods(mods, || {
            let mut stacks = [
                Some(crate::__private::part_tests::stack(
                    "minecraft:potion",
                    1,
                    0,
                )),
                None,
                None,
                None,
                None,
            ];
            let mut cancelled = false;
            {
                let mut journal = HookListJournal::new(
                    HookListStorage::Fixed(&mut stacks),
                    |value: &Option<ItemStack>| Some(value.clone()),
                    |value| {
                        if value.as_ref().is_some_and(|stack| stack.count() > 1) {
                            return Err(StackWriteError::CountExceeded { maximum: 1 });
                        }
                        Ok(value.clone())
                    },
                );
                let mut slots = HookList::new(&mut journal, "potion_brew_pre", "stacks");
                dispatch_potion_brew_pre(&mut NoHost, [1, 2, 3], &mut slots, &mut cancelled);
            }
            assert_eq!(cancelled, cancel);
            assert_eq!(stacks[1], None);
            assert_eq!(
                stacks[0],
                Some(crate::__private::part_tests::stack(
                    "minecraft:diamond",
                    1,
                    0
                ))
            );
            // The engine decides whether to apply the pre slots. Dispatch itself
            // threads the caller's owned values through either contract.
            {
                let mut journal = HookListJournal::new(
                    HookListStorage::Fixed(&mut stacks),
                    |value: &Option<ItemStack>| Some(value.clone()),
                    |value| Ok(value.clone()),
                );
                let mut slots = HookList::new(&mut journal, "potion_brew_post", "stacks");
                dispatch_potion_brew_post(&mut NoHost, [1, 2, 3], &mut slots);
            }
            assert_eq!(
                stacks[0],
                Some(crate::__private::part_tests::stack(
                    "minecraft:diamond",
                    1,
                    0
                ))
            );
            assert_eq!(
                stacks[1],
                Some(crate::__private::part_tests::stack(
                    "minecraft:emerald",
                    1,
                    0
                ))
            );
        });
        assert_eq!(*seen.lock().unwrap(), ["pre", "post minecraft:diamond"]);
    }
}

#[cfg(test)]
use super::*;

#[cfg(test)]
use crate::testing;
