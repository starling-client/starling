use super::*;
pub(super) fn roll_facts<'a>(
    query: &'a query::LootQuery,
    place: Option<&'a LootPlace>,
    entity: Option<EntityFacts<'a>>,
    biome: Option<&'a str>,
    luck: f32,
) -> LootContext<'a> {
    LootContext {
        queried_table_id: query.id(),
        queried_stack: query.queried_stack.as_ref(),
        entity: entity.map(|e| e.this),
        damage: entity.and_then(|e| e.damage),
        player: entity.and_then(|e| e.player),
        interacting_entity: None,
        tool: query
            .tool
            .as_ref()
            .map(|stack| stack as &dyn LootToolAccess),
        block: None,
        luck,
        biome,
        place,
    }
}

impl<'a> LootContext<'a> {
    /// Queried id, including the unknown id before the root completes.
    pub fn table_id(&self) -> &'a str {
        self.queried_table_id.unwrap_or(query::UNKNOWN_LOOT_TABLE)
    }
}
impl LootData {
    /// LootTable.getRandomItems with the given random (its state is
    /// advanced), luck, the origin and level its location checks read, and, for
    /// fishing, whether the hook is in open water.
    pub fn roll<R: RandomSource + Clone>(
        &self,
        table: &str,
        random: &mut R,
        luck: f32,
        location: Option<LootLocation<'_>>,
        open_water: Option<bool>,
    ) -> Vec<LootStack> {
        let Some(json) = self.tables.get(table) else {
            return Vec::new();
        };
        let mut c = Context {
            query: query::current_query(),
            place: current_native_loot_place(),
            data: self,
            local_bindings: None,
            random: random.clone(),
            biome: None,
            location,
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
    /// still reads `open_water`; `location` is the hook's position and level.
    pub fn roll_as<R: RandomSource + Clone>(
        &self,
        table: &str,
        random: &mut R,
        biome: Option<&str>,
        location: Option<LootLocation<'_>>,
        open_water: Option<bool>,
        params: &EntityLootParams,
    ) -> Vec<LootStack> {
        let Some(json) = self.tables.get(table) else {
            return Vec::new();
        };
        let mut c = Context {
            query: query::current_query(),
            place: current_native_loot_place(),
            data: self,
            local_bindings: None,
            random: random.clone(),
            biome,
            location,
            visited: Vec::new(),
            luck: params.luck,
            open_water,
            entity: Some(params.into()),
            enchanted_level: None,
        };
        let items = c.roll_root(Some(table), json);
        *random = c.random;
        items
    }

    /// Roll an unnamed root, preparing it once before drawing and sharing any
    /// caller-supplied queried parameters.
    pub fn roll_inline<R: RandomSource + Clone>(
        &self,
        table: &Value,
        random: &mut R,
    ) -> Vec<LootStack> {
        let prepared = match self.bindings.prepare_inline_table(table) {
            Ok(prepared) => prepared,
            Err(error) => {
                log::warn!("inline loot table refused: {error}");
                return Vec::new();
            }
        };
        self.roll_prepared_inline(&prepared, random)
    }

    /// Roll an already-prepared inline loot table, sharing any caller-supplied queried parameters.
    pub fn roll_prepared_inline<R: RandomSource + Clone>(
        &self,
        table: &types::PreparedInlineTable,
        random: &mut R,
    ) -> Vec<LootStack> {
        let mut c = Context {
            query: query::current_query(),
            place: current_native_loot_place(),
            data: self,
            local_bindings: Some(table.rules()),
            random: random.clone(),
            biome: None,
            location: None,
            visited: Vec::new(),
            luck: 0.0,
            open_water: None,
            entity: None,
            enchanted_level: None,
        };
        let items = c.roll_root(None, table.value());
        *random = c.random;
        items
    }
}
#[cfg(test)]
#[path = "loot_query_tests.rs"]
mod tests;
