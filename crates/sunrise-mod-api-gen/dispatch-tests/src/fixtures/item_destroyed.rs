/// An item part that pushes one stack onto `event.drops` and records the damage type it heard.
struct DestroyedSpillPart {
    add_item: &'static str,
    count: u32,
    seen_damage: Arc<Mutex<Vec<String>>>,
}
impl ItemPart for DestroyedSpillPart {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[hook_ids::item_destroyed]
    }
    fn item_destroyed(&self, _: &mut dyn ServerHost, event: &mut ItemDestroyedEvent<'_, '_>) {
        self.seen_damage
            .lock()
            .unwrap()
            .push(event.damage_type.to_owned());
        // A refused stack leaves the list as it was; the part goes on.
        let _ = event.drops.push(ItemStack {
            item: Arc::from(self.add_item),
            count: self.count,
        });
    }
}

/// Installs three parts on the destroyed item, in order: 2 diamonds, a 999-count
/// `invalid:overstack` (above any stack limit), then 3 emeralds.
fn install_destroyed_spill_parts(seen: &Arc<Mutex<Vec<String>>>) {
    let part = |add_item, count| -> &'static dyn ItemPart {
        leak(DestroyedSpillPart {
            add_item,
            count,
            seen_damage: seen.clone(),
        })
    };
    install_parts(Parts {
        items: vec![
            part("minecraft:diamond", 2),
            part("invalid:overstack", 999),
            part("minecraft:emerald", 3),
        ],
        ..Parts::default()
    });
}

/// Runs the dispatch for a destroyed `test:pouch` with no vanilla spill and `validate` as the
/// engine's check of each submitted drop, returning the final drops as `(item, count)`.
fn dispatch_pouch_destroyed(
    validate: impl Fn(&ItemStack) -> Result<ItemStack, StackWriteError>,
) -> Vec<(String, u32)> {
    let mut host = TestHost;
    let stack = ItemStack {
        item: Arc::from("test:pouch"),
        count: 1,
    };
    let mut drops = Vec::new();
    let mut journal = HookListJournal::new(
        HookListStorage::Resizable(&mut drops),
        |stack: &ItemStack| Some(stack.clone()),
        validate,
    );
    let mut list = HookList::new(&mut journal, "item_destroyed", "drops");
    dispatch_item_destroyed(
        &mut host,
        &stack,
        ItemEntityRef,
        "minecraft:lava",
        &mut list,
    );
    destroyed_drops(&drops)
}

fn destroyed_drops(drops: &[ItemStack]) -> Vec<(String, u32)> {
    drops
        .iter()
        .map(|stack| (stack.item.to_string(), stack.count))
        .collect()
}

fn expected_drops(drops: &[(&str, u32)]) -> Vec<(String, u32)> {
    drops
        .iter()
        .map(|&(item, count)| (item.to_owned(), count))
        .collect()
}

#[test]
fn item_destroyed_refuses_only_the_drop_the_boundary_check_refuses() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    install_destroyed_spill_parts(&seen);

    // The engine's check converts each submitted stack and refuses a count above the stack limit.
    let checked = Mutex::new(Vec::new());
    let drops = dispatch_pouch_destroyed(|stack| {
        checked
            .lock()
            .unwrap()
            .push(destroyed_drops(std::slice::from_ref(stack)));
        if stack.count > 64 {
            Err(StackWriteError::CountExceeded)
        } else {
            Ok(stack.clone())
        }
    });

    // Every part heard the hook, and each stack was checked on its own.
    assert_eq!(*seen.lock().unwrap(), vec!["minecraft:lava".to_owned(); 3]);
    assert_eq!(
        checked.into_inner().unwrap(),
        vec![
            expected_drops(&[("minecraft:diamond", 2)]),
            expected_drops(&[("invalid:overstack", 999)]),
            expected_drops(&[("minecraft:emerald", 3)]),
        ]
    );
    // The refused stack never joined the list; the parts before and after it keep their drops.
    assert_eq!(
        drops,
        expected_drops(&[("minecraft:diamond", 2), ("minecraft:emerald", 3)])
    );
}

#[test]
fn item_destroyed_keeps_every_parts_drops_the_validator_accepts() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    install_destroyed_spill_parts(&seen);

    // The dispatch refuses nothing itself: with a check that accepts everything, even the
    // overstack stays, so the refusal above is the validator's alone.
    let drops = dispatch_pouch_destroyed(|stack| Ok(stack.clone()));

    assert_eq!(*seen.lock().unwrap(), vec!["minecraft:lava".to_owned(); 3]);
    assert_eq!(
        drops,
        expected_drops(&[
            ("minecraft:diamond", 2),
            ("invalid:overstack", 999),
            ("minecraft:emerald", 3),
        ])
    );
}
