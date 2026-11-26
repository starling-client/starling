/// First writes to each field, independently: moving an item need not clone its stack.
#[derive(Default)]
pub(super) struct KeptItemEntity {
    stack: Option<(ItemStack, u64)>,
    pickup_delay: Option<i32>,
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
        if let Some((stack, revision)) = self.stack {
            item.stack = stack;
            item.stack_revision = revision;
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
    /// Sets an item place's stack. A drained place discards the item on commit.
    pub(super) fn journal_set_item_entity(&mut self, id: u32, stack: ItemStack) -> bool {
        let Some(index) = self.dropped.iter().position(|item| item.id == id) else {
            return false;
        };
        let Some(level) = self.transactions.level_frame(self.dim) else {
            return self.native_set_item_entity(index, stack);
        };
        if stack.count == 0 {
            level.after.push(After::Emptied(id));
        }
        self.journal_replace_item_stack(index, stack);
        true
    }

    /// `ItemEntity.setItem`: set the validated resident slot without the item-place
    /// removal action. An empty stack is discarded by `ItemEntity.tick` instead.
    pub(super) fn journal_set_item_entity_at(&mut self, index: usize, stack: Option<ItemStack>) {
        let stack = stack
            .filter(|stack| &*stack.item != "minecraft:air")
            .unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
        self.journal_replace_item_stack(index, stack);
    }

    fn journal_replace_item_stack(&mut self, index: usize, stack: ItemStack) {
        let (item, kept) = self.journal_item_fields(index);
        if let Some(kept) = kept {
            kept.stack
                .get_or_insert_with(|| (item.stack.clone(), item.stack_revision));
        }
        item.stack = stack;
        item.stack_revision = item.stack_revision.wrapping_add(1);
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
        item.pickup_delay = ticks;
    }

    pub(super) fn journal_set_item_target(&mut self, index: usize, target: Option<[i32; 4]>) {
        let (item, kept) = self.journal_item_fields(index);
        if let Some(kept) = kept {
            kept.target.get_or_insert(item.target);
        }
        item.target = target;
    }
}
