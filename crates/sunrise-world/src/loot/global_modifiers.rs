//! Prepared, data-enabled modifiers and their one-pass roll capability.
#[cfg(test)]
mod tests;
use super::{LootContext, LootStack};
use crate::conditions::LoadConditions;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use sunrise_core::identifier::Identifier;

/// Borrowed access to the evaluator that owns this root roll. No operation starts
/// another root, changes its parameters, or selects a different random sequence.
pub trait ModifierRoll {
    fn facts(&self) -> LootContext<'_>;
    fn next_int(&mut self) -> i32;
    fn next_int_bound(&mut self, bound: i32) -> i32;
    fn next_long(&mut self) -> i64;
    fn next_float(&mut self) -> f32;
    fn next_double(&mut self) -> f64;
    fn next_boolean(&mut self) -> bool;
    fn next_gaussian(&mut self) -> f64;
    fn condition(&mut self, condition: &Value) -> bool;
    fn roll_raw(&mut self, table: &str) -> Result<Vec<LootStack>, String>;
}

/// A validated candidate replaces the preceding list; failures leave it intact.
/// The implementation must not split, clamp or filter the result after applying.
pub trait ModifierApply: Send + Sync {
    fn apply(
        &self,
        context: &mut dyn ModifierRoll,
        generated: &[LootStack],
    ) -> Result<Vec<LootStack>, String>;
}

/// Metadata captured by the owning type decoder, before sorting the generation.
pub struct DecodedModifier {
    pub priority: i32,
    pub condition: Option<Value>,
    pub apply: Arc<dyn ModifierApply>,
}

/// A selected source, including read failures: an unreadable winning file must
/// mask a lower file just as malformed JSON or false load conditions do.
pub struct ModifierFile {
    pub id: String,
    pub pack: String,
    pub path: String,
    pub read: Box<dyn FnOnce() -> Result<Vec<u8>, String>>,
}

/// One independently refused instance; the rest of this generation still loads.
#[derive(Debug)]
pub struct ModifierDiagnostic {
    pub id: String,
    pub pack: String,
    pub path: String,
    pub reason: String,
}
impl std::fmt::Display for ModifierDiagnostic {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            out,
            "loot modifier {} (pack {}, file {}): {}",
            self.id, self.pack, self.path, self.reason
        )
    }
}

struct PreparedModifier {
    id: String,
    kind: String,
    decoded: DecodedModifier,
}

/// Immutable, already-sorted instances retained by the owning server snapshot.
#[derive(Default)]
pub struct GlobalLootModifiers {
    instances: Vec<PreparedModifier>,
}
impl GlobalLootModifiers {
    /// Files arrive in ascending pack priority. Select a winner before parsing
    /// or evaluating its load conditions; never fall back to a losing file.
    pub fn load(
        files: impl IntoIterator<Item = ModifierFile>,
        conditions: &LoadConditions,
        decode: impl Fn(&str, &str, &Value) -> Result<DecodedModifier, String>,
        mut report: impl FnMut(ModifierDiagnostic),
    ) -> Self {
        let mut selected = BTreeMap::new();
        for file in files {
            selected.insert(file.id.clone(), file);
        }
        let mut instances = Vec::new();
        for (_, file) in selected {
            let prepared = (|| {
                Identifier::parse(&file.id).map_err(|error| format!("id: {error}"))?;
                let bytes = (file.read)().map_err(|error| format!("read: {error}"))?;
                let data: Value =
                    serde_json::from_slice(&bytes).map_err(|error| format!("JSON: {error}"))?;
                if !conditions.allows_file_isolated(&data, &file.path)? {
                    log::debug!(
                        "loot modifier {} (pack {}, file {}) skipped by load conditions",
                        file.id,
                        file.pack,
                        file.path
                    );
                    return Ok(None);
                }
                let kind = data
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or("type: expected a serializer id")?;
                let kind = Identifier::parse(kind)
                    .map_err(|error| format!("type: {error}"))?
                    .to_string();
                let decoded = decode(&file.id, &kind, &data)?;
                Ok::<_, String>(Some(PreparedModifier {
                    id: file.id.clone(),
                    kind,
                    decoded,
                }))
            })();
            match prepared {
                Ok(Some(instance)) => instances.push(instance),
                Ok(None) => {}
                Err(reason) => report(ModifierDiagnostic {
                    id: file.id,
                    pack: file.pack,
                    path: file.path,
                    reason,
                }),
            }
        }
        instances.sort_by(|a, b| {
            b.decoded
                .priority
                .cmp(&a.decoded.priority)
                .then_with(|| a.id.cmp(&b.id))
        });
        Self { instances }
    }

    /// Selected instance ids in application order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.instances.iter().map(|instance| instance.id.as_str())
    }

    /// The only check a root needs before constructing any callback capability.
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Run once after the root splitter, before consumption or container shuffle.
    /// Conditions and callbacks share the selected random. Their draws are never
    /// rolled back, even when the candidate list is refused.
    pub fn apply(
        &self,
        context: &mut dyn ModifierRoll,
        mut generated: Vec<LootStack>,
    ) -> Vec<LootStack> {
        for instance in &self.instances {
            if instance
                .decoded
                .condition
                .as_ref()
                .is_some_and(|condition| !context.condition(condition))
            {
                continue;
            }
            match instance.decoded.apply.apply(context, &generated) {
                Ok(candidate) => generated = candidate,
                Err(error) => log::warn!(
                    "loot modifier {} (type {}) on table {} refused: {}",
                    instance.id,
                    instance.kind,
                    context.facts().table_id(),
                    error
                ),
            }
        }
        generated
    }
}

/// The built-in add_table operation, constructed from already-decoded fields.
/// It follows the same prepared-instance interface without crossing a mod ABI.
pub struct AddTableModifier {
    pub table: String,
}
impl ModifierApply for AddTableModifier {
    fn apply(
        &self,
        context: &mut dyn ModifierRoll,
        generated: &[LootStack],
    ) -> Result<Vec<LootStack>, String> {
        let appended = context.roll_raw(&self.table)?;
        Ok(append_raw(generated, appended))
    }
}

/// Append raw output unchanged, shared by the engine and retained API instance.
pub fn append_raw<S: Clone>(generated: &[S], appended: Vec<S>) -> Vec<S> {
    let mut result = Vec::with_capacity(generated.len() + appended.len());
    result.extend_from_slice(generated);
    result.extend(appended);
    result
}
