//! Registered damage value effects, resolved when enchantment content loads.
use crate::{EntityRef, Identifier, ItemStack, LivingRef, ServerHost};
use std::sync::Arc;

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// Context of one value node in ordinary enchantment damage computation.
// exhaustive: the game constructs the hit context with a literal; mods only read it
pub struct EnchantmentDamageContext<'a> {
    /// The actual attacker of this hit.
    pub attacker: LivingRef,
    /// The actual victim of this hit.
    pub victim: EntityRef,
    /// Weapon snapshot taken before the hit.
    pub weapon: &'a ItemStack,
    /// Enchantment level on the weapon.
    pub level: u32,
    /// This effect node's content, converted once to NBT at load, including its type and custom fields.
    pub config: &'a sunrise_core::nbt::Tag,
}

/// A registered value transformation. Called only for nodes naming this effect.
/// The host is read-only; callbacks may be nested and must not retain hit context.
pub trait EnchantmentDamageEffect: Send + Sync + 'static {
    /// Transform the value at this node, in vanilla enchantment and node order.
    /// NaN retains the input. Other results, including infinities, are clamped
    /// to 0..=1024 before the next node, bounding per-unit built-in work.
    fn apply(
        &self,
        host: &dyn ServerHost,
        context: &EnchantmentDamageContext<'_>,
        value: f32,
    ) -> f32;
}

/// Startup registrar for namespaced enchantment damage effects.
pub trait EnchantmentDamageEffects {
    /// Register a non-vanilla identifier. Duplicate and vanilla identifiers are refused.
    fn register(
        &mut self,
        id: Identifier,
        effect: Arc<dyn EnchantmentDamageEffect>,
    ) -> Result<(), String>;
}

type DamageEffectEntry = (Identifier, Arc<dyn EnchantmentDamageEffect>);

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// Engine-owned storage behind the startup registrar.
#[derive(Default)]
#[non_exhaustive]
pub struct EnchantmentDamageRegistry {
    entries: Vec<DamageEffectEntry>,
}

struct DamageEffectEdits<'a> {
    accepted: &'a [DamageEffectEntry],
    tail: &'a mut Vec<DamageEffectEntry>,
}

impl EnchantmentDamageEffects for DamageEffectEdits<'_> {
    fn register(
        &mut self,
        id: Identifier,
        effect: Arc<dyn EnchantmentDamageEffect>,
    ) -> Result<(), String> {
        if id.to_string().starts_with("minecraft:")
            || self
                .accepted
                .iter()
                .chain(self.tail.iter())
                .any(|(key, _)| key == &id)
        {
            return Err(format!(
                "invalid or duplicate enchantment damage effect {id}"
            ));
        }
        self.tail.push((id, effect));
        Ok(())
    }
}

impl EnchantmentDamageEffects for EnchantmentDamageRegistry {
    fn register(
        &mut self,
        id: Identifier,
        effect: Arc<dyn EnchantmentDamageEffect>,
    ) -> Result<(), String> {
        DamageEffectEdits {
            accepted: &[],
            tail: &mut self.entries,
        }
        .register(id, effect)
    }
}

impl EnchantmentDamageRegistry {
    /// Append this listener's effects only if its registration succeeds.
    // hidden: the engine stages one listener at a time; mods receive only the registrar trait.
    #[doc(hidden)]
    pub fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn EnchantmentDamageEffects) -> bool,
    ) {
        let mut tail = Vec::new();
        if listen(&mut DamageEffectEdits {
            accepted: &self.entries,
            tail: &mut tail,
        }) {
            self.entries.append(&mut tail);
        } else {
            let owner = crate::content::crash::current_native_mod();
            for (_, effect) in tail {
                crate::runtime::discard_registration(
                    (
                        crate::generated::HookId::RegisterEnchantmentDamageEffects,
                        "register_enchantment_damage_effects",
                    ),
                    owner,
                    effect,
                );
            }
        }
    }

    /// Engine-owned registration entries, in slot order.
    // hidden: the engine consumes registrations after startup; not part of the mod API
    #[doc(hidden)]
    pub fn into_entries(self) -> Vec<(Identifier, Arc<dyn EnchantmentDamageEffect>)> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Identity;
    impl EnchantmentDamageEffect for Identity {
        fn apply(&self, _: &dyn ServerHost, _: &EnchantmentDamageContext<'_>, value: f32) -> f32 {
            value
        }
    }
    #[test]
    fn damage_effect_registration_refuses_vanilla_and_duplicate_ids() {
        let mut effects = EnchantmentDamageRegistry::default();
        assert!(
            effects
                .register(
                    Identifier::parse("minecraft:add").unwrap(),
                    Arc::new(Identity)
                )
                .is_err()
        );
        assert!(
            effects
                .register(
                    Identifier::parse("test:damage").unwrap(),
                    Arc::new(Identity)
                )
                .is_ok()
        );
        assert!(
            effects
                .register(
                    Identifier::parse("test:damage").unwrap(),
                    Arc::new(Identity)
                )
                .is_err()
        );
        assert_eq!(effects.into_entries().len(), 1);
    }
}
