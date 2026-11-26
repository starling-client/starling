//! Atomic edit spanning a resident block entity and a dropped item entity.

// ANCHOR: block_entity_transaction
use mod_api::{BlockEntityScope, ItemEntityRef, ItemStack, TransactionError};

pub fn update_with_item(
    tick: &mut BlockEntityScope<'_>,
    item: ItemEntityRef,
    replacement: ItemStack,
    progress: i32,
) -> Result<(), TransactionError<&'static str>> {
    tick.transaction(|tick, _| {
        if !item
            .set_stack(tick.host(), Some(replacement))
            .map_err(|_| "invalid stack")?
        {
            return Err("dropped-item write was refused");
        }
        let changed = tick
            .with_view(|mut view| view.set_data_slot(0, progress))
            .ok_or("subject is unavailable")?;
        if !changed {
            return Err("progress was not updated");
        }
        Ok(())
    })
}
// ANCHOR_END: block_entity_transaction
