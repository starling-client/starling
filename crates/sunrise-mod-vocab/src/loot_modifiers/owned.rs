use super::*;
use std::sync::atomic::AtomicBool;

pub(crate) fn owned_serializer(
    owner: &'static str,
    index: usize,
    serializer: Arc<dyn GlobalLootModifierSerializer>,
) -> Arc<dyn GlobalLootModifierSerializer> {
    Arc::new(OwnedSerializer {
        owner,
        index,
        serializer,
        poisoned: AtomicBool::new(false),
    })
}

struct OwnedSerializer {
    owner: &'static str,
    index: usize,
    serializer: Arc<dyn GlobalLootModifierSerializer>,
    poisoned: AtomicBool,
}
impl GlobalLootModifierSerializer for OwnedSerializer {
    fn decode(
        &self,
        data: &Value,
        load: &dyn DataLoadContext,
    ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
        let (instance, priority, condition) = crate::callback_owner::call(
            self.owner,
            self.index,
            &self.poisoned,
            "loot modifier serializer",
            "global_loot_modifier_decode",
            || -> Result<_, LootModifierError> {
                let instance = self.serializer.decode(data, load)?;
                // These resource methods execute mod code too. Capture them in
                // the same owning load and owner scope, never on the roll path.
                let priority = instance.priority()?;
                let condition = instance.condition()?;
                Ok((instance, priority, condition))
            },
        )
        .map_err(|reason| LootModifierError::new("$", reason))??;
        Ok(Arc::new(OwnedModifier {
            owner: self.owner,
            index: self.index,
            instance,
            priority,
            condition,
            poisoned: AtomicBool::new(false),
        }))
    }
}

struct OwnedModifier {
    owner: &'static str,
    index: usize,
    instance: Arc<dyn GlobalLootModifier>,
    priority: i32,
    condition: Option<Value>,
    poisoned: AtomicBool,
}
impl GlobalLootModifier for OwnedModifier {
    fn priority(&self) -> Result<i32, LootModifierError> {
        Ok(self.priority)
    }
    fn condition(&self) -> Result<Option<Value>, LootModifierError> {
        Ok(self.condition.clone())
    }
    fn apply(
        &self,
        context: &mut dyn LootModifierContext,
        generated: Vec<LootStack>,
    ) -> Result<Vec<LootStack>, LootModifierError> {
        crate::callback_owner::call(
            self.owner,
            self.index,
            &self.poisoned,
            "loot modifier",
            "global_loot_modifier_apply",
            || self.instance.apply(context, generated),
        )
        .map_err(|reason| LootModifierError::new("apply", reason))?
    }
}
