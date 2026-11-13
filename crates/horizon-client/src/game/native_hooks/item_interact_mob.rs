use super::*;

impl Game<'_> {
    /// an item fallback after a native mob's ordinary interaction passed.
    pub(in crate::game) fn native_item_interact_mob(
        &mut self,
        entity: i32,
        kind: &str,
        stack: &ItemStack,
        swing: &mut Option<bool>,
    ) {
        self.fire_item_interact_mob(entity, kind, stack, swing);
    }
}
