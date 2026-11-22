use super::*;
impl<'a, R: RandomSource> Context<'a, R> {
    pub(super) fn roll_facts(&self, seed: i64) -> LootContext<'a> {
        LootContext {
            queried_table_id: self.query.id(),
            queried_stack: self.query.queried_stack.clone(),
            entity: self.entity.map(|e| &e.this),
            damage: self.entity.and_then(|e| e.damage.as_ref()),
            player: self.entity.and_then(|e| e.player.as_ref()),
            tool: self.query.tool.as_ref().map(|stack| LootTool {
                item: stack.item.clone(),
                enchantments: stack.enchantments.clone(),
            }),
            block: None,
            luck: self.luck,
            biome: self.biome,
            seed,
            place: current_native_loot_place(),
        }
    }
}

impl LootContext<'_> {
    /// Queried id, including the unknown fallback before the root completes.
    pub fn table_id(&self) -> &str {
        self.queried_table_id
            .as_deref()
            .unwrap_or(query::UNKNOWN_LOOT_TABLE)
    }
}
impl LootData {
    /// LootTable.getRandomItems with the given random (its state is
    /// advanced), luck and, for fishing, whether the hook is in open water.
    pub fn roll<R: RandomSource + Clone>(
        &self,
        table: &str,
        random: &mut R,
        luck: f32,
        biome: Option<&str>,
        open_water: Option<bool>,
    ) -> Vec<LootStack> {
        let Some(json) = self.tables.get(table) else {
            return Vec::new();
        };
        let mut c = Context {
            query: query::current_query(),
            data: self,
            random: random.clone(),
            biome,
            visited: Vec::new(),
            luck,
            open_water,
            entity: None,
            enchanted_level: None,
        };
        let items = c.roll_root(Some(table), json);
        *random = c.random;
        items
    }

    /// [`Self::roll`] with the entity parameters of `params` (a fishing hook
    /// as `this` and its owner as the attacker), and `params.luck` as the
    /// luck. The entity predicates read them, and a hook's `in_open_water`
    /// still reads `open_water`.
    pub fn roll_as<R: RandomSource + Clone>(
        &self,
        table: &str,
        random: &mut R,
        biome: Option<&str>,
        open_water: Option<bool>,
        params: &EntityLootParams,
    ) -> Vec<LootStack> {
        let Some(json) = self.tables.get(table) else {
            return Vec::new();
        };
        let mut c = Context {
            query: query::current_query(),
            data: self,
            random: random.clone(),
            biome,
            visited: Vec::new(),
            luck: params.luck,
            open_water,
            entity: Some(params),
            enchanted_level: None,
        };
        let items = c.roll_root(Some(table), json);
        *random = c.random;
        items
    }

    /// Roll an unnamed root, sharing any caller-supplied queried parameters.
    pub fn roll_inline<R: RandomSource + Clone>(
        &self,
        table: &Value,
        random: &mut R,
    ) -> Vec<LootStack> {
        let mut c = Context {
            query: query::current_query(),
            data: self,
            random: random.clone(),
            biome: None,
            visited: Vec::new(),
            luck: 0.0,
            open_water: None,
            entity: None,
            enchanted_level: None,
        };
        let items = c.roll_root(None, table);
        *random = c.random;
        items
    }
}
#[cfg(test)]
#[path = "loot_query_tests.rs"]
mod tests;
