//! Resolve predicate/item-modifier tags once, without entering codec-owned payloads.
use super::*;
impl super::super::LootData {
    /// Edit and validate private table replacements before the one publication.
    /// A refused replacement leaves this snapshot and its retained codecs intact.
    pub fn apply_native_table_loads(
        &mut self,
        context: &crate::native_load_context::LoadContext,
    ) -> io::Result<()> {
        let mut ids: Vec<_> = self.tables.ids().collect();
        ids.sort();
        let mut tables = HashMap::new();
        for id in ids {
            let mut table = self.tables[id].clone();
            super::super::native_loot_table_load(context, id, &mut table);
            super::super::validate_loot_table(&table).map_err(|reason| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("native loot_table {id}: {reason}"),
                )
            })?;
            self.holder_sets
                .walk(&format!("native loot_table {id}"), &mut table, &self.tags)?;
            table_fields(&mut table, &self.tags, &self.types);
            tables.insert(id.to_owned(), table);
        }
        self.types.validate(
            tables.iter().map(|(id, value)| (id.as_str(), value)),
            &self.predicates,
            &self.modifiers,
        )?;
        self.tables = Arc::new(tables.into_iter().collect());
        self.refresh_copy_requirements();
        Ok(())
    }
    pub(in crate::loot) fn prepare_types(&mut self) -> io::Result<()> {
        let tags = &self.tags;
        let types = &self.types;
        let mut tables = HashMap::new();
        for id in self.tables.ids() {
            let mut table = self.tables[id].clone();
            table_fields(&mut table, tags, types);
            tables.insert(id.to_owned(), table);
        }
        self.tables = Arc::new(tables.into_iter().collect());
        for value in Arc::make_mut(&mut self.predicates).values_mut() {
            typed(value, false, tags, types);
        }
        for value in self.modifiers.values_mut() {
            typed(value, true, tags, types);
        }
        self.validate_types()?;
        self.refresh_copy_requirements();
        Ok(())
    }
}
use super::super::loot_tags::BoundTags as Tags;
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
    // validation pass, and registered payloads remain opaque.
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
