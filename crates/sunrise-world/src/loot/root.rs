//! Non-raw loot table completion, distinct from nested raw evaluation.
use super::*;

impl<R: RandomSource + Clone> Context<'_, R> {
    /// Every non-raw entry point completes here with its original context and
    /// random still available. Nested entries call `table_raw` instead: their
    /// functions run before the root splitter and they never complete a query.
    pub(super) fn roll_root(&mut self, id: Option<&str>, table: &Value) -> Vec<LootStack> {
        let mut items = Vec::new();
        self.table_raw(id.unwrap_or("<root>"), table, &mut |context, stack| {
            context.data.split(stack, &mut items)
        });
        // Every non-raw roll completes after the splitter with its own
        // context and random, before the caller consumes the list.
        self.query.finish(id);
        let generation = &self.data.global_modifiers;
        if generation.is_empty() {
            return items;
        }
        generation.apply(self, items)
    }
}

impl<R: RandomSource + Clone> global_modifiers::ModifierRoll for Context<'_, R> {
    fn facts(&self) -> LootContext<'_> {
        self.roll_facts(0)
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
        Context::condition(self, condition)
    }
    fn roll_raw(&mut self, id: &str) -> Result<Vec<LootStack>, String> {
        let table = self
            .data
            .tables
            .get(id)
            .ok_or_else(|| format!("table: missing loot table {id}"))?;
        let mut output = Vec::new();
        self.table_raw(id, table, &mut |_, stack| output.push(stack));
        Ok(output)
    }
}
