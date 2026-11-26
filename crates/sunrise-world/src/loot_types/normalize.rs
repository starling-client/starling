//! Resolve holder sets and tags without entering codec-owned payloads.
use super::super::loot_tags::BoundTags as Tags;
use super::*;

impl super::super::LootData {
    pub(in crate::loot) fn prepare_types(
        &mut self,
        types: &Types,
        native: Option<&crate::native_load_context::LoadContext>,
    ) -> io::Result<()> {
        let mut ids: Vec<_> = self.tables.ids().collect();
        ids.sort_unstable();
        let mut tables = HashMap::with_capacity(ids.len());
        for id in ids {
            let mut table = self.tables[id].clone();
            if let Some(context) = native {
                super::super::native_loot_table_load(context, id, &mut table);
                super::super::validate_loot_table(&table).map_err(|reason| {
                    invalid(&format!("native loot_table {id}"), "loot_table", reason)
                })?;
            }
            let path = if native.is_some() {
                format!("native loot_table {id}")
            } else {
                resource("loot_table", id)
            };
            self.holder_sets
                .walk(&path, &mut table, &self.tags, types, None)?;
            table_fields(&mut table, &self.tags, types);
            tables.insert(id.to_owned(), table);
        }
        self.tables = Arc::new(tables.into_iter().collect());
        for value in Arc::make_mut(&mut self.predicates).values_mut() {
            typed(value, false, &self.tags, types);
        }
        for value in Arc::make_mut(&mut self.modifiers).values_mut() {
            typed(value, true, &self.tags, types);
        }
        for enchantment in &mut self.enchantments {
            for effect in Arc::make_mut(&mut enchantment.effects)
                .get_mut("minecraft:equipment_drops")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if let Some(req) = effect
                    .get_mut("requirements")
                    .filter(|value| !value.is_null())
                {
                    typed(req, false, &self.tags, types);
                }
            }
        }
        self.bindings = types.bind_generation(
            self.tables.clone(),
            self.predicates.clone(),
            self.modifiers.clone(),
            self.enchantments
                .iter()
                .map(|e| (e.name.clone(), e.effects.clone()))
                .collect(),
            Normalization {
                holder_sets: self.holder_sets.clone(),
                tags: self.tags.clone(),
            },
        )?;
        self.refresh_copy_requirements();
        Ok(())
    }
}

pub(super) fn normalize_table(value: &mut Value, tags: &Tags, types: &Types) {
    table_fields(value, tags, types);
}

pub(super) fn normalize_condition(value: &mut Value, tags: &Tags, types: &Types) {
    typed(value, false, tags, types);
}

fn table_fields(value: &mut Value, tags: &Tags, types: &Types) {
    fields(value, tags, types);
    for pool in value
        .get_mut("pools")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        fields(pool, tags, types);
        for entry in pool
            .get_mut("entries")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            container(entry, tags, types);
        }
    }
}
fn container(value: &mut Value, tags: &Tags, types: &Types) {
    fields(value, tags, types);
    for child in value
        .get_mut("children")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        container(child, tags, types);
    }
    if value["type"] == "minecraft:loot_table" && value["value"].is_object() {
        table_fields(&mut value["value"], tags, types);
    }
}
fn fields(value: &mut Value, tags: &Tags, types: &Types) {
    if let Some(condition) = value.get_mut("condition") {
        typed(condition, false, tags, types);
    }
    if let Some(modifier) = value.get_mut("modifier") {
        typed(modifier, true, tags, types);
    }
}
fn typed(value: &mut Value, function: bool, tags: &Tags, types: &Types) {
    if let Some(id) = value.as_str() {
        if function && id.starts_with('#') {
            set(value, true, tags, types);
        } else if let Ok(id) = sunrise_core::identifier::Identifier::parse(id) {
            *value = Value::String(id.to_string());
        }
        return;
    }
    if let Some(id) = value.get("type").and_then(Value::as_str)
        && let Ok(id) = sunrise_core::identifier::Identifier::parse(id)
    {
        value["type"] = Value::String(id.to_string());
    }
    if function && value.is_array() {
        set(value, true, tags, types);
        return;
    }
    let kind = value["type"].as_str().unwrap_or("");
    let conditional = shapes::contains(kind, true) || types.conditional(kind);
    // Locate only schema-owned edges; scalar/text decoding belongs to the
    // binding pass, and registered payloads remain opaque.
    let children = shapes::children(value, function, kind)
        .into_iter()
        .map(|child| (child.kind, child.pointer))
        .collect::<Vec<_>>();
    for (kind, pointer) in children {
        if let Some(child) = value.pointer_mut(&pointer) {
            match kind {
                shapes::ChildKind::Function => typed(child, true, tags, types),
                shapes::ChildKind::Condition => typed(child, false, tags, types),
                shapes::ChildKind::Set(function) => set(child, function, tags, types),
                shapes::ChildKind::Table => {}
            }
        }
    }
    if function
        && conditional
        && let Some(condition) = value.get_mut("condition").filter(|value| !value.is_null())
    {
        typed(condition, false, tags, types);
    }
}

fn set(value: &mut Value, function: bool, tags: &Tags, types: &Types) {
    if let Some(tag) = value.as_str().and_then(|id| id.strip_prefix('#')) {
        let Ok(tag) = sunrise_core::identifier::Identifier::parse(tag) else {
            return;
        };
        let registry = if function {
            "item_modifier"
        } else {
            "predicate"
        };
        let ids = tags
            .get(registry)
            .and_then(|tags| tags.get(tag.to_string().as_str()));
        *value = Value::Array(
            ids.into_iter()
                .flatten()
                .cloned()
                .map(Value::String)
                .collect(),
        );
    } else if !value.is_array() {
        *value = Value::Array(vec![std::mem::take(value)]);
    }
    for term in value.as_array_mut().into_iter().flatten() {
        typed(term, function, tags, types);
    }
}
