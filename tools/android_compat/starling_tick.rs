//! Starling's port to Sunrise's API: code written for Horizon's, which handed a block entity's
//! slots, compound and data slots (`ModBlockEntityFields`) to a tick or a menu button to change in
//! place, runs here on a copy of them taken from Sunrise's view; what it changed is written back and
//! the chunk marked unsaved, as Horizon's reply did.
use mod_api::{BlockEntityScope, BlockEntityTickEvent, ServerHost};

/// The block entity as Horizon's tick had it.
pub(crate) type OldEntity = mod_api::item::containers::ModBlockEntityFields;

/// Run `body` on a copy of the ticking block entity (its host, the copy, its position).
pub(crate) fn run(
    event: &mut BlockEntityTickEvent<'_>,
    body: impl FnOnce(&mut dyn ServerHost, &mut OldEntity, [i32; 3]),
) {
    run_scope(&mut event.subject, body);
}

/// Run `body` on a copy of the block entity `subject` lends (a tick's, a menu button's).
pub(crate) fn run_scope(
    subject: &mut BlockEntityScope<'_>,
    body: impl FnOnce(&mut dyn ServerHost, &mut OldEntity, [i32; 3]),
) {
    let Some((handle, pos, before)) = subject.with_view(|view| {
        let items = (0..view.item_slots() as u32)
            .map(|slot| view.item_slot(slot).and_then(|stack| stack.to_owned()))
            .collect();
        let entity = OldEntity {
            items,
            nbt: view.nbt().clone(),
            data: view.data_slots().to_vec(),
        };
        (view.handle(), view.position(), entity)
    }) else {
        return;
    };
    let mut after = before.clone();
    body(subject.host(), &mut after, pos);
    let mut changed = false;
    subject.with_view(|mut view| {
        for (key, value) in after.nbt.iter() {
            if before.nbt.get(key) != Some(value) && view.set_nbt(key.clone(), Some(value.clone())) {
                changed = true;
            }
        }
        for (key, _) in before.nbt.iter() {
            if after.nbt.get(key).is_none() && view.set_nbt(key.clone(), None) {
                changed = true;
            }
        }
        for (slot, stack) in after.items.iter().enumerate() {
            if before.items.get(slot) != Some(stack) {
                let _ = view.set_item_slot(slot as u32, stack.clone());
                changed = true;
            }
        }
        if after.data != before.data && view.set_data_slots(&after.data) {
            changed = true;
        }
    });
    if changed {
        let _ = handle.set_changed(subject.host());
    }
}
