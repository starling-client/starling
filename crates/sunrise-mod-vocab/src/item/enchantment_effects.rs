//! Persistent map codecs for vanilla's two enchantment effect registries.
use crate::callback_owner::{CallbackOwner, CallbackValue};
use crate::{EntityRef, EquipmentSlot, Identifier, ItemStack, LivingRef, RandomSource, ServerHost};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

/// A map codec in `Registries.ENCHANTMENT_VALUE_EFFECT_TYPE`.
pub trait EnchantmentValueEffectCodec: Send + Sync + 'static {
    /// Decode the winning resource map once. An error refuses the definition.
    /// The game owns the `type` dispatch field.
    fn decode(&self, fields: &Value) -> Result<Arc<dyn EnchantmentValueEffect>, String>;
}

/// One immutable, decoded `EnchantmentValueEffect`.
pub trait EnchantmentValueEffect: Send + Sync + 'static {
    /// Transform the current value. The draw is from the random vanilla lends to this
    /// stage, not the level random that tested its requirements. Return the unmodified
    /// input to leave it alone. The result goes to the next effect without normalization.
    fn process(&self, level: u32, random: &mut dyn RandomSource, value: f32) -> f32;
}

/// A map codec in `Registries.ENCHANTMENT_ENTITY_EFFECT_TYPE`.
pub trait EnchantmentEntityEffectCodec: Send + Sync + 'static {
    /// Decode the winning resource map once. An error refuses the definition.
    /// The game owns the `type` dispatch field.
    fn decode(&self, fields: &Value) -> Result<Arc<dyn EnchantmentEntityEffect>, String>;
}

/// One immutable, decoded `EnchantmentEntityEffect`.
pub trait EnchantmentEntityEffect: Send + Sync + 'static {
    /// Apply to the entity selected by the component and its requirements.
    fn apply(&self, host: &mut dyn ServerHost, context: &EnchantmentEntityEffectContext<'_>);
}

// no wire: this call-scoped view lends the enchanted stack to a native effect.
/// `EnchantmentEntityEffect.apply`: the enchanted item in use and the affected entity.
// exhaustive: the engine makes this view; mods only read it.
pub struct EnchantmentEntityEffectContext<'a> {
    /// Level of the enchantment supplying this effect.
    pub level: u32,
    /// The actual enchanted item, not the holder's current hand if this is a projectile.
    pub item: &'a ItemStack,
    /// Living holder of the item, when present.
    pub holder: Option<LivingRef>,
    /// The equipment slot in `EnchantedItemInUse`, when there is one.
    pub slot: Option<EquipmentSlot>,
    /// The entity selected by this effect; it may have just died.
    pub affected: EntityRef,
    /// The effect's application position.
    pub position: [f64; 3],
    /// The victim of a post-attack effect.
    pub victim: Option<EntityRef>,
    /// `DamageSource.getEntity()` for a post-attack effect, when present.
    pub causing: Option<EntityRef>,
    /// `DamageSource.getDirectEntity()` for a post-attack effect, when present.
    pub direct: Option<EntityRef>,
}

/// Registers codecs before enchantment definitions are bound.
pub trait EnchantmentEffects {
    /// Register a map codec in the value registry. Only an occupied ID is refused.
    fn register_value(
        &mut self,
        id: Identifier,
        codec: Arc<dyn EnchantmentValueEffectCodec>,
    ) -> Result<(), String>;

    /// Register a map codec in the entity registry. Only an occupied ID is refused.
    fn register_entity(
        &mut self,
        id: Identifier,
        codec: Arc<dyn EnchantmentEntityEffectCodec>,
    ) -> Result<(), String>;
}

const REGISTRATION: (crate::generated::HookId, &str) = (
    crate::generated::HookId::RegisterEnchantmentEffects,
    "register_enchantment_effects",
);
const VALUE_EFFECT: &str = "enchantment value effect";
const ENTITY_EFFECT: &str = "enchantment entity effect";

// A bad decode quarantines the codec; a bad invocation quarantines its effect kind,
// without making the same definitions impossible to reload.
struct Codec<T: ?Sized> {
    decoder: CallbackValue<Arc<T>>,
    effect: CallbackOwner,
}

// no wire: process-local map codecs.
/// Engine-owned table of effect types.
// hidden: the engine decodes effect types through it; mods register through the registrar.
// exhaustive: the fields are private.
#[doc(hidden)]
#[derive(Default)]
pub struct EnchantmentEffectRegistry {
    values: HashMap<String, Codec<dyn EnchantmentValueEffectCodec>>,
    entities: HashMap<String, Codec<dyn EnchantmentEntityEffectCodec>>,
}

/// The value kinds registered by vanilla, under their canonical IDs.
// hidden: the engine's decoder tells vanilla's kinds from mods' kinds by it.
#[doc(hidden)]
pub fn builtin_enchantment_value_kind(id: &str) -> bool {
    matches!(
        id,
        "minecraft:add"
            | "minecraft:all_of"
            | "minecraft:exponential"
            | "minecraft:multiply"
            | "minecraft:remove_binomial"
            | "minecraft:set"
    )
}

/// The entity kinds registered by vanilla, under their canonical IDs.
// hidden: the engine's decoder tells vanilla's kinds from mods' kinds by it.
#[doc(hidden)]
pub fn builtin_enchantment_entity_kind(id: &str) -> bool {
    matches!(
        id,
        "minecraft:all_of"
            | "minecraft:apply_mob_effect"
            | "minecraft:change_item_damage"
            | "minecraft:damage_entity"
            | "minecraft:explode"
            | "minecraft:ignite"
            | "minecraft:apply_impulse"
            | "minecraft:apply_exhaustion"
            | "minecraft:play_sound"
            | "minecraft:replace_block"
            | "minecraft:replace_disk"
            | "minecraft:run_function"
            | "minecraft:set_block_properties"
            | "minecraft:spawn_particles"
            | "minecraft:summon_entity"
    )
}

struct EffectEdits<'a> {
    accepted: &'a EnchantmentEffectRegistry,
    tail: &'a mut EnchantmentEffectRegistry,
}

impl EnchantmentEffects for EffectEdits<'_> {
    fn register_value(
        &mut self,
        id: Identifier,
        codec: Arc<dyn EnchantmentValueEffectCodec>,
    ) -> Result<(), String> {
        let id = id.to_string();
        if builtin_enchantment_value_kind(&id)
            || self.accepted.values.contains_key(&id)
            || self.tail.values.contains_key(&id)
        {
            return Err(format!("duplicate {VALUE_EFFECT} {id}"));
        }
        let owner = registration_owner()?;
        self.tail.values.insert(
            id,
            Codec {
                effect: CallbackOwner::for_mod(owner.id()),
                decoder: CallbackValue::new(
                    owner,
                    VALUE_EFFECT,
                    "enchantment_value_codec_drop",
                    codec,
                ),
            },
        );
        Ok(())
    }

    fn register_entity(
        &mut self,
        id: Identifier,
        codec: Arc<dyn EnchantmentEntityEffectCodec>,
    ) -> Result<(), String> {
        let id = id.to_string();
        if builtin_enchantment_entity_kind(&id)
            || self.accepted.entities.contains_key(&id)
            || self.tail.entities.contains_key(&id)
        {
            return Err(format!("duplicate {ENTITY_EFFECT} {id}"));
        }
        let owner = registration_owner()?;
        self.tail.entities.insert(
            id,
            Codec {
                effect: CallbackOwner::for_mod(owner.id()),
                decoder: CallbackValue::new(
                    owner,
                    ENTITY_EFFECT,
                    "enchantment_entity_codec_drop",
                    codec,
                ),
            },
        );
        Ok(())
    }
}

fn registration_owner() -> Result<CallbackOwner, String> {
    crate::content::crash::current_native_mod()
        .and_then(CallbackOwner::new)
        .ok_or_else(|| "enchantment codec registration needs an active native mod".into())
}

impl EnchantmentEffectRegistry {
    /// Stage one listener's declarations; a refused listener publishes none of them.
    pub fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn EnchantmentEffects) -> bool,
    ) {
        let mut tail = Self::default();
        if listen(&mut EffectEdits {
            accepted: self,
            tail: &mut tail,
        }) {
            self.values.extend(tail.values);
            self.entities.extend(tail.entities);
        } else {
            crate::runtime::discard_registration(
                REGISTRATION,
                crate::content::crash::current_native_mod(),
                tail,
            );
        }
    }

    /// Resolve a custom value effect in the selected enchantment definition.
    pub fn decode_value(
        &self,
        id: &str,
        fields: &Value,
    ) -> Result<BoundEnchantmentValueEffect, String> {
        let codec = self
            .values
            .get(id)
            .ok_or_else(|| format!("unknown {VALUE_EFFECT} {id}"))?;
        let fields = effect_codec_fields(fields)?;
        let owner = codec.decoder.owner();
        let value = owner.call(VALUE_EFFECT, "enchantment_value_decode", || {
            codec.decoder.value().decode(&fields)
        })??;
        Ok(BoundEnchantmentValueEffect(Arc::new(CallbackValue::new(
            codec.effect.clone(),
            VALUE_EFFECT,
            "enchantment_value_drop",
            value,
        ))))
    }

    /// Resolve a custom entity effect in the selected enchantment definition.
    pub fn decode_entity(
        &self,
        id: &str,
        fields: &Value,
    ) -> Result<BoundEnchantmentEntityEffect, String> {
        let codec = self
            .entities
            .get(id)
            .ok_or_else(|| format!("unknown {ENTITY_EFFECT} {id}"))?;
        let fields = effect_codec_fields(fields)?;
        let owner = codec.decoder.owner();
        let value = owner.call(ENTITY_EFFECT, "enchantment_entity_decode", || {
            codec.decoder.value().decode(&fields)
        })??;
        Ok(BoundEnchantmentEntityEffect(Arc::new(CallbackValue::new(
            codec.effect.clone(),
            ENTITY_EFFECT,
            "enchantment_entity_drop",
            value,
        ))))
    }
}

fn effect_codec_fields(effect: &Value) -> Result<Value, String> {
    let mut fields = effect
        .as_object()
        .ok_or("enchantment effect must be an object")?
        .clone();
    fields.remove("type");
    Ok(Value::Object(fields))
}

// no wire: process-local decoded behavior.
/// Bound value effect with its native callback owner.
// hidden: the engine holds decoded effects; mods implement `EnchantmentValueEffect`.
// exhaustive: the field is private.
#[doc(hidden)]
#[derive(Clone)]
pub struct BoundEnchantmentValueEffect(Arc<CallbackValue<Arc<dyn EnchantmentValueEffect>>>);
impl BoundEnchantmentValueEffect {
    /// Invoke the decoded handler. Only an unavailable or panicking callback is skipped.
    pub fn process(&self, level: u32, random: &mut dyn RandomSource, value: f32) -> f32 {
        self.0
            .owner()
            .call(VALUE_EFFECT, "enchantment_value_process", || {
                self.0.value().process(level, random, value)
            })
            .unwrap_or(value)
    }
}

// no wire: process-local decoded behavior.
/// Bound entity effect with its native callback owner.
// hidden: the engine holds decoded effects; mods implement `EnchantmentEntityEffect`.
// exhaustive: the field is private.
#[doc(hidden)]
#[derive(Clone)]
pub struct BoundEnchantmentEntityEffect(Arc<CallbackValue<Arc<dyn EnchantmentEntityEffect>>>);
impl BoundEnchantmentEntityEffect {
    /// Invoke this bound handler only, using the owning mod's native invocation scope.
    pub fn apply(&self, host: &mut dyn ServerHost, context: &EnchantmentEntityEffectContext<'_>) {
        let _ = self
            .0
            .owner()
            .call(ENTITY_EFFECT, "enchantment_entity_apply", || {
                self.0.value().apply(host, context)
            });
    }
}
