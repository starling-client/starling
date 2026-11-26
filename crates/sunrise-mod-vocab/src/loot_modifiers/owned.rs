use super::*;
use crate::callback_owner::{CallbackOwner, CallbackValue};

pub(crate) fn owned_serializer(
    owner: CallbackOwner,
    serializer: Arc<dyn GlobalLootModifierSerializer>,
) -> Arc<dyn GlobalLootModifierSerializer> {
    Arc::new(OwnedSerializer(CallbackValue::new(
        owner,
        "loot modifier serializer",
        "global_loot_modifier_serializer_drop",
        serializer,
    )))
}

struct OwnedSerializer(CallbackValue<Arc<dyn GlobalLootModifierSerializer>>);
impl GlobalLootModifierSerializer for OwnedSerializer {
    fn decode(
        &self,
        data: &Value,
        load: &dyn DataLoadContext,
    ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
        let owner = self.0.owner();
        let instance = owner
            .call(
                "loot modifier serializer",
                "global_loot_modifier_decode",
                || self.0.value().decode(data, load),
            )
            .map_err(|reason| LootModifierError::new("$", reason))??;
        let instance = CallbackValue::new(
            owner.clone(),
            "loot modifier serializer",
            "global_loot_modifier_drop",
            instance,
        );
        // Metadata is mod code, too; capture it in the owning load, never during a roll.
        let (priority, condition) = owner
            .call(
                "loot modifier serializer",
                "global_loot_modifier_metadata",
                || {
                    let modifier = instance.value();
                    Ok::<_, LootModifierError>((modifier.priority()?, modifier.condition()?))
                },
            )
            .map_err(|reason| LootModifierError::new("$", reason))??;
        Ok(Arc::new(OwnedModifier {
            instance,
            priority,
            condition,
        }))
    }
}

struct OwnedModifier {
    instance: CallbackValue<Arc<dyn GlobalLootModifier>>,
    priority: i32,
    condition: Option<Value>,
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
        generated: &mut LootList<'_>,
    ) -> Result<(), LootModifierError> {
        self.instance
            .owner()
            .call("loot modifier", "global_loot_modifier_apply", || {
                self.instance.value().apply(context, generated)
            })
            .map_err(|reason| LootModifierError::new("apply", reason))?
    }
}
