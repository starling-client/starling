//! Starling's part of packets_entity.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Starling: Entity.DATA_CUSTOM_NAME (2) and DATA_CUSTOM_NAME_VISIBLE (3) of any entity, for
    /// the names the game draws over what it does not model (`remote_name_tags`).
    pub(super) fn remote_names(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        for (index, _, value) in values {
            match (index, value) {
                (2, Value::OptText(t)) => {
                    r.names.entry(id).or_insert((None, false)).0 = t.as_ref().map(remote_text);
                }
                (3, Value::Bool(v)) => r.names.entry(id).or_insert((None, false)).1 = *v,
                _ => {}
            }
        }
    }
}
