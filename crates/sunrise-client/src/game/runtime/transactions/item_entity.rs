/// First writes to each field, independently: moving an item need not clone its stack.
#[derive(Default)]
pub(super) struct KeptItemEntity {
    stack: Option<ItemStack>,
    pickup_delay: Option<i64>,
    target: Option<Option<[i32; 4]>>,
    pub(super) velocity: Option<Vec3>,
}

impl KeptItemEntity {
    fn merge(&mut self, child: Self) {
        self.stack = self.stack.take().or(child.stack);
        self.pickup_delay = self.pickup_delay.or(child.pickup_delay);
        self.target = self.target.or(child.target);
        self.velocity = self.velocity.or(child.velocity);
    }

    fn restore(self, item: &mut sunrise_client::entities::ItemEntity) {
        if let Some(stack) = self.stack {
            item.stack = stack;
        }
        if let Some(delay) = self.pickup_delay {
            item.pickup_delay = delay;
        }
        if let Some(target) = self.target {
            item.target = target;
        }
        if let Some(velocity) = self.velocity {
            item.velocity = velocity;
        }
    }
}

impl Game<'_> {
    /// Sets the stack of `id` for an item place. A caller that already has a
    /// validated resident slot uses `journal_set_item_entity_at` instead.
    pub(super) fn journal_set_item_entity(&mut self, id: u32, stack: ItemStack) -> bool {
        self.dropped
            .iter()
            .position(|item| item.id == id)
            .is_some_and(|index| self.journal_set_item_entity_at(index, stack))
    }

    /// An item emptied in a transaction stays, empty, until root commit removes it.
    pub(super) fn journal_set_item_entity_at(&mut self, index: usize, stack: ItemStack) -> bool {
        let Some(level) = self.transactions.level_frame(self.dim) else {
            return self.native_set_item_entity(index, stack);
        };
        let item = &mut self.dropped[index];
        level
            .item_entities
            .entry(item.join)
            .or_default()
            .stack
            .get_or_insert_with(|| item.stack.clone());
        if stack.count == 0 {
            level.after.push(After::Emptied(item.id));
        }
        item.stack = stack;
        true
    }

    /// Lend an item in the active level and the first-write journal of its innermost open frame.
    pub(super) fn journal_item_fields(
        &mut self,
        index: usize,
    ) -> (
        &mut sunrise_client::entities::ItemEntity,
        Option<&mut KeptItemEntity>,
    ) {
        let item = &mut self.dropped[index];
        let kept = self
            .transactions
            .level_frame(self.dim)
            .map(|level| level.item_entities.entry(item.join).or_default());
        (item, kept)
    }

    pub(super) fn journal_set_item_pickup_delay(&mut self, index: usize, ticks: i32) {
        let (item, kept) = self.journal_item_fields(index);
        if let Some(kept) = kept {
            kept.pickup_delay.get_or_insert(item.pickup_delay);
        }
        item.pickup_delay = i64::from(ticks);
    }

    pub(super) fn journal_set_item_target(&mut self, index: usize, target: Option<[i32; 4]>) {
        let (item, kept) = self.journal_item_fields(index);
        if let Some(kept) = kept {
            kept.target.get_or_insert(item.target);
        }
        item.target = target;
    }
}
