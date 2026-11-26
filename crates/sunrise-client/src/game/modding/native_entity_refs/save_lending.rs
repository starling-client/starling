//! Save visibility while authoritative rows are lent out.
use super::*;

impl Game<'_> {
    /// Keep a held-out entity inaccessible until its copy has been stored back.
    pub(in crate::game) fn with_entity_held_out<R>(
        &mut self,
        id: u32,
        action: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let _save_lease = sunrise_client::join_seq::visibility::PassLease::enter();
        if mod_api::__private::mods().is_empty() {
            return action(self);
        }
        self.held_out.push(id);
        let result = action(self);
        self.held_out.pop();
        result
    }

    /// Let a shared callback read the current stored row, without letting a later
    /// mutable callback touch the entity before its tick finishes storing it back.
    pub(in crate::game) fn with_held_entity_read<R>(
        &mut self,
        id: u32,
        action: impl FnOnce(&Self) -> R,
    ) -> R {
        let held = self.held_out.last() == Some(&id);
        if held {
            self.held_out.pop();
        }
        let result = action(self);
        if held {
            self.held_out.push(id);
        }
        result
    }
}
