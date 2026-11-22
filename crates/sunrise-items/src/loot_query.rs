use super::*;

impl<R: RandomSource> LootContext<'_, R> {
    pub(super) fn stock_condition(&self, c: &Value) -> Option<bool> {
        if !matches!(
            c["type"].as_str(),
            Some("neoforge:loot_table_id" | "neoforge:can_item_perform_ability")
        ) {
            return None;
        }
        let tool = (c["type"].as_str() == Some("neoforge:can_item_perform_ability"))
            .then(|| {
                self.held
                    .filter(|s| s.count > 0 && &*s.item != "minecraft:air")
                    .map(loot_of)
            })
            .flatten();
        sunrise_world::loot::query::condition(c, &self.query, tool.as_ref(), |item, ability| {
            self.items.vanilla_can_perform(item, ability)
        })
    }
    /// Non-raw completion splits after table functions, retaining the caller's
    /// context and random. Nested entries evaluate raw and never complete a query.
    pub(super) fn roll_root(
        &self,
        id: Option<&str>,
        table: &Value,
        random: &mut R,
    ) -> Vec<ItemStack> {
        let raw = self.evaluate(table, random);
        self.complete_root(id, raw, random, None)
    }
    pub(super) fn complete_root(
        &self,
        id: Option<&str>,
        raw: Vec<ItemStack>,
        random: &mut R,
        dynamic: Option<&[ItemStack]>,
    ) -> Vec<ItemStack> {
        let stacks = self.items.split_loot_stacks(raw);
        self.query.finish(id);
        let generation = &self.items.global_modifiers;
        if generation.is_empty() && dynamic.is_none() {
            return stacks;
        }
        let dynamic = dynamic.unwrap_or(&[]);
        let stacks = self.expand_dynamic(stacks, dynamic, true);
        if generation.is_empty() {
            return stacks;
        }
        let generated = stacks.iter().map(loot_of).collect();
        let mut scope = ModifierScope {
            context: self,
            random,
            dynamic,
        };
        generation
            .apply(&mut scope, generated)
            .iter()
            .map(ItemStack::from_loot)
            .collect()
    }
    fn expand_dynamic(
        &self,
        stacks: Vec<ItemStack>,
        dynamic: &[ItemStack],
        split: bool,
    ) -> Vec<ItemStack> {
        if !stacks.iter().any(|stack| stack.item.starts_with(DYNAMIC)) {
            return stacks;
        }
        stacks
            .into_iter()
            .flat_map(|stack| {
                if stack.item.starts_with(DYNAMIC) {
                    if split {
                        self.items.split_loot_stacks(dynamic.to_vec())
                    } else {
                        dynamic.to_vec()
                    }
                } else {
                    vec![stack]
                }
            })
            .collect()
    }
    /// The context a mod's own loot kind sees in this engine: the block and
    /// the tool it has, and a seed from the roll's random. The entity, the
    /// damage, the player and the biome belong to the main loot engine's
    /// rolls; they are None here (docs/modding/src/how-to/native-mods.md).
    #[cfg(test)]
    pub(super) fn native_loot_context(
        &self,
        random: &mut R,
    ) -> sunrise_world::loot::LootContext<'_> {
        self.roll_facts(random.next_long())
    }
    pub(super) fn roll_facts(&self, seed: i64) -> sunrise_world::loot::LootContext<'_> {
        sunrise_world::loot::LootContext {
            queried_table_id: self.query.id(),
            queried_stack: self.query.queried_stack.clone(),
            entity: self.target.map(|(entity, _)| entity),
            damage: None,
            player: None,
            block: (!self.block.is_empty()).then_some(self.block),
            tool: self.held.map(|h| sunrise_world::loot::LootTool {
                item: h.item.to_string(),
                enchantments: h
                    .enchantments()
                    .iter()
                    .map(|(id, lvl)| (id.to_string(), *lvl as i32))
                    .collect(),
            }),
            luck: 0.0,
            biome: None,
            seed,
            place: sunrise_world::loot::current_native_loot_place(),
        }
    }
}

impl ItemStack {
    /// Convert loot facts using the same component write path as native loot functions.
    /// Caller consumption: signed empty counts become zero without removing their list position.
    pub fn from_loot(value: &sunrise_world::loot::LootStack) -> Self {
        let mut stack = Self {
            item: Arc::from(value.item.as_str()),
            count: 1,
            damage: 0,
            extra: None,
        };
        let before = loot_of(&stack);
        apply_loot(&before, value, &mut stack, |_, _| None);
        stack
    }
}

impl Items {
    /// `LootTable.createStackSplitter`, also used after resolving dynamic drops.
    pub(super) fn split_loot_stacks(&self, stacks: Vec<ItemStack>) -> Vec<ItemStack> {
        // Stack limits are at least one: singles and empty positions need no lookup.
        if !stacks.iter().any(|stack| {
            stack.count > 1
                && !stack.item.starts_with(DYNAMIC)
                && stack.count > crate::bundle::max_stack(self, stack)
        }) {
            return stacks;
        }
        let mut out = Vec::with_capacity(stacks.len());
        for stack in stacks {
            // A dynamic marker is not an item. Its supplied stacks are split
            // when block_drops resolves it, never the marker's own count.
            if stack.count <= 1 || stack.item.starts_with(DYNAMIC) {
                out.push(stack);
                continue;
            }
            let max = crate::bundle::max_stack(self, &stack);
            let mut count = stack.count;
            while count > max {
                let mut part = stack.clone();
                part.count = max;
                out.push(part);
                count -= max;
            }
            out.push(ItemStack { count, ..stack });
        }
        out
    }

    pub(super) fn scoped_block_roll(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        roll: BlockLootRoll<'_>,
        random: &mut impl RandomSource,
        sequences: Option<(&mut crate::random_sequences::RandomSequences, i64)>,
    ) -> Vec<ItemStack> {
        let Some(block) = registry.state(state) else {
            return vec![];
        };
        let table = self.block_loot_table(registry, state);
        let Some(id) = table else {
            return vec![];
        };
        let Some(table) = self.loot.get(&id) else {
            return vec![];
        };
        sunrise_world::loot::query::with_query(sunrise_world::loot::query::current_query(), || {
            // LootTable.getRandomItems draws from the table's random sequence.
            if let (Some(seq), Some(sequences)) = (
                table.get("random_sequence").and_then(Value::as_str),
                sequences,
            ) {
                let random = sequences.0.get(seq, sequences.1);
                return self.roll_block_table(
                    &id,
                    table,
                    &block.name,
                    &block.properties,
                    roll,
                    random,
                );
            }
            self.roll_block_table(&id, table, &block.name, &block.properties, roll, random)
        })
    }
    /// `Block.dropFromBlockInteractLootTable`: the `block_interact` table `id`
    /// for a right click on `state` (its `match_block` conditions see the
    /// state), from the table's random sequence when it has one.
    /// `interacting` is `INTERACTING_ENTITY`, the player or mob that clicked;
    /// `location` reads the world around `ORIGIN`, the block's centre.
    pub fn interact_loot(
        &self,
        id: &str,
        registry: &BlockRegistry,
        state: BlockStateId,
        interacting: Option<crate::items::EntityTarget<'_>>,
        location: Option<&dyn Fn(&Value) -> Option<bool>>,
        random: &mut impl RandomSource,
        sequences: &mut crate::random_sequences::RandomSequences,
        seed: i64,
    ) -> Vec<ItemStack> {
        let (Some(table), Some(block)) = (self.loot.get(id), registry.state(state)) else {
            return vec![];
        };
        sunrise_world::loot::query::with_query(sunrise_world::loot::query::current_query(), || {
            match table.get("random_sequence").and_then(Value::as_str) {
                Some(sequence) => {
                    let random = sequences.get(sequence, seed);
                    self.interact_roll(
                        id,
                        table,
                        &block.name,
                        &block.properties,
                        interacting,
                        location,
                        random,
                    )
                }
                None => self.interact_roll(
                    id,
                    table,
                    &block.name,
                    &block.properties,
                    interacting,
                    location,
                    random,
                ),
            }
        })
    }
}

#[cfg(test)]
#[path = "loot_query_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "loot_split_tests.rs"]
mod split_tests;

/// One capability lending the original evaluator and selected random through
/// the root pass. Raw sub-rolls retain all borrowed parameters and visited guards.
struct ModifierScope<'s, 'a, R> {
    context: &'s LootContext<'a, R>,
    random: &'s mut R,
    dynamic: &'s [ItemStack],
}
impl<R: RandomSource> sunrise_world::loot::global_modifiers::ModifierRoll
    for ModifierScope<'_, '_, R>
{
    fn facts(&self) -> sunrise_world::loot::LootContext<'_> {
        self.context.roll_facts(0)
    }
    fn next_int(&mut self) -> i32 {
        self.random.next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.random.next_int_bound(bound).unwrap_or(0)
    }
    fn next_long(&mut self) -> i64 {
        self.random.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.random.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.random.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.random.next_boolean()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.random.next_gaussian()
    }
    fn condition(&mut self, condition: &Value) -> bool {
        self.context.condition(condition, self.random)
    }
    fn roll_raw(
        &mut self,
        id: &str,
    ) -> std::result::Result<Vec<sunrise_world::loot::LootStack>, String> {
        let context = self.context;
        let table = context
            .items
            .loot
            .get(id)
            .ok_or_else(|| format!("table: missing loot table {id}"))?;
        let Some(_guard) = NestedRoll::enter(id) else {
            return Ok(Vec::new());
        };
        // Raw output retains emitted empties all the way through nested tables.
        // The original root's output policy does not filter a raw sub-roll.
        let raw = LootContext {
            query: context.query.clone(),
            items: context.items,
            block: context.block,
            properties: context.properties,
            held: context.held,
            killed_by_player: context.killed_by_player,
            explosion_radius: context.explosion_radius,
            this_entity: context.this_entity,
            entity_levels: context.entity_levels,
            block_levels: context.block_levels,
            target: context.target,
            target_entity: context.target_entity,
            interacting_entity: context.interacting_entity,
            location: context.location,
            keep_empty: true,
        }
        .evaluate(table, self.random);
        Ok(context
            .expand_dynamic(raw, self.dynamic, false)
            .iter()
            .map(loot_of)
            .collect())
    }
}
