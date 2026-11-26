use super::*;
use crate::loot_candidate::{LootCandidate, LootCandidates};
use sunrise_world::loot::{LootBlockView, LootListAccess, LootStackAccess, LootToolAccess};

impl LootToolAccess for ItemStack {
    fn item_id(&self) -> &str {
        &self.item
    }

    fn enchantment_at(&self, index: usize) -> Option<(&str, i32)> {
        self.enchantments()
            .get(index)
            .map(|(id, level)| (id.as_ref(), *level as i32))
    }
}

impl sunrise_world::loot::LootAbilityTool for ItemStack {
    fn item_id(&self) -> &str {
        &self.item
    }

    fn is_empty(&self) -> bool {
        self.count == 0 || &*self.item == "minecraft:air"
    }

    fn resident_scalars(&self) -> Option<(std::num::NonZeroU32, Option<u32>)> {
        let count = std::num::NonZeroU32::new(self.count)?;
        let damage = self
            .carries_component(None, "minecraft:damage")
            .then_some(self.damage);
        Some((count, damage))
    }

    fn encode_components(&self) -> Option<sunrise_core::nbt::Tag> {
        crate::item_data::encode_extra_components(self)
    }
}

impl<R: RandomSource> LootContext<'_, R> {
    pub(super) fn stock_condition(&self, c: &Value) -> Option<bool> {
        sunrise_world::loot::query::condition(
            c,
            &self.query,
            self.held
                .map(|tool| tool as &dyn sunrise_world::loot::LootAbilityTool),
            self.items,
            |item, ability| self.items.vanilla_can_perform(item, ability),
        )
    }
    /// Non-raw completion splits after table functions, retaining the caller's
    /// context and random. Nested entries evaluate raw and never complete a query.
    pub(super) fn roll_root(
        &self,
        id: Option<&str>,
        table: &Value,
        random: &mut R,
    ) -> Vec<ItemStack> {
        let raw = self.evaluate_candidates(table, random);
        self.complete_root(id, raw, random, None)
    }
    pub(super) fn complete_root(
        &self,
        id: Option<&str>,
        raw: Vec<LootCandidate>,
        random: &mut R,
        dynamic: Option<&[ItemStack]>,
    ) -> Vec<ItemStack> {
        let stacks = self.items.split_loot_candidates(raw);
        self.query.finish(id);
        let generation = &self.items.global_modifiers;
        if generation.is_empty() && dynamic.is_none() {
            return stacks
                .into_iter()
                .map(|candidate| candidate.into_item_stack(self.items))
                .collect();
        }
        let dynamic = dynamic.unwrap_or(&[]);
        let generated = self.expand_dynamic(stacks, dynamic, true);
        if generation.is_empty() {
            return generated
                .into_iter()
                .map(|candidate| candidate.into_item_stack(self.items))
                .collect();
        }
        let mut scope = ModifierScope {
            context: self,
            random,
            dynamic,
        };
        let mut generated = LootCandidates(generated);
        generation.apply(&mut scope, &mut generated);
        generated
            .0
            .into_iter()
            .map(|candidate| candidate.into_item_stack(self.items))
            .collect()
    }
    fn expand_dynamic(
        &self,
        stacks: Vec<LootCandidate>,
        dynamic: &[ItemStack],
        split: bool,
    ) -> Vec<LootCandidate> {
        if !stacks.iter().any(LootCandidate::is_dynamic_marker) {
            return stacks;
        }
        stacks
            .into_iter()
            .flat_map(|stack| {
                if stack.is_dynamic_marker() {
                    let dynamic = dynamic
                        .iter()
                        .cloned()
                        .map(LootCandidate::from_stack)
                        .collect();
                    if split {
                        self.items.split_loot_candidates(dynamic)
                    } else {
                        dynamic
                    }
                } else {
                    vec![stack]
                }
            })
            .collect()
    }
    /// Borrow the parameters this evaluator has; random is lent separately.
    pub(super) fn roll_facts(&self) -> sunrise_world::loot::LootContext<'_> {
        sunrise_world::loot::LootContext {
            queried_table_id: self.query.id(),
            queried_stack: self.query.queried_stack.as_ref(),
            entity: self.target.map(|(entity, _)| entity),
            damage: None,
            player: None,
            interacting_entity: self.interacting_entity.map(|(entity, _)| entity),
            block: (!self.block.is_empty())
                .then(|| LootBlockView::new(self.block, self.properties)),
            tool: self.held.map(|tool| tool as &dyn LootToolAccess),
            luck: 0.0,
            biome: None,
            place: self.place.as_deref(),
        }
    }
}

impl ItemStack {
    /// Convert loot facts using the same component write path as native loot functions.
    /// Caller consumption: signed empty counts become zero without removing their list position.
    pub fn from_loot(value: &sunrise_world::loot::LootStack) -> Self {
        LootCandidate::from_loot_stack(value)
    }
}

impl Items {
    /// `LootTable.createStackSplitter`, also used after resolving dynamic drops.
    pub(super) fn split_loot_candidates(&self, stacks: Vec<LootCandidate>) -> Vec<LootCandidate> {
        let mut out = Vec::with_capacity(stacks.len());
        for stack in stacks {
            stack.split_into(self, &mut out);
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
    /// `held` is `TOOL`, the item used to interact (if any);
    /// `location` reads the world around `ORIGIN`, the block's centre.
    #[allow(clippy::too_many_arguments)]
    pub fn interact_loot(
        &self,
        id: &str,
        registry: &BlockRegistry,
        state: BlockStateId,
        interacting: Option<crate::items::EntityTarget<'_>>,
        held: Option<&ItemStack>,
        location: Option<sunrise_world::loot::LootLocation<'_>>,
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
                        held,
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
                    held,
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
impl<R: RandomSource> ModifierScope<'_, '_, R> {
    fn eval_raw(&mut self, id: &str) -> std::result::Result<Vec<LootCandidate>, String> {
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
            place: context.place.clone(),
            items: context.items,
            local_bindings: None,
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
        .evaluate_candidates(table, self.random);
        Ok(context.expand_dynamic(raw, self.dynamic, false))
    }
}
impl<R: RandomSource> sunrise_world::loot::global_modifiers::ModifierRoll
    for ModifierScope<'_, '_, R>
{
    fn facts(&self) -> sunrise_world::loot::LootContext<'_> {
        self.context.roll_facts()
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
    fn condition(&mut self, condition: &sunrise_world::loot::types::PreparedCondition) -> bool {
        self.context.condition_with_bindings(
            condition.value(),
            self.random,
            Some(condition.rules()),
        )
    }
    fn roll_raw(
        &mut self,
        id: &str,
        visit: &mut dyn FnMut(&dyn LootStackAccess),
    ) -> std::result::Result<(), String> {
        for candidate in self.eval_raw(id)? {
            visit(&candidate);
        }
        Ok(())
    }
    fn append_raw(
        &mut self,
        id: &str,
        generated: &mut dyn LootListAccess,
    ) -> std::result::Result<(), String> {
        let target = (generated as &mut dyn std::any::Any)
            .downcast_mut::<LootCandidates>()
            .ok_or("foreign loot output list")?;
        target.0.extend(self.eval_raw(id)?);
        Ok(())
    }
}
