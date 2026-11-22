//! The data components that tune an item's behaviour (`food`, `consumable`,
//! `tool`, `equippable`, `attribute_modifiers`, ...), from their stream form to
//! the codec form. See [`super::raw`] for the rest.
use super::*;
use crate::buf::Reader;
use crate::item::{ATTRIBUTE_OPERATIONS, EQUIPMENT_SLOT_GROUPS};
use sunrise_world::snbt_text::to_snbt;

use sunrise_core::use_animation::NAMES as USE_ANIMATION;
const SWING: [&str; 3] = ["none", "whack", "stab"];
/// `EquipmentSlot` by `id` (not declaration order).
const SLOT: [&str; 8] = [
    "mainhand", "feet", "legs", "chest", "head", "offhand", "body", "saddle",
];

fn int_with<O: Ops>(r: &mut Reader) -> Option<i32> {
    r.varint().ok()
}
fn float_with<O: Ops>(r: &mut Reader) -> Option<f32> {
    r.f32().ok()
}
fn boolean_with<O: Ops>(r: &mut Reader) -> Option<bool> {
    r.bool().ok()
}
fn ident_with<O: Ops>(r: &mut Reader) -> Option<O::Value> {
    identifier_with::<O>(&r.string(32767).ok()?)
}
/// `Codec.xxx.optionalFieldOf(name, default)` for a float.
fn float_or_with<O: Ops>(rec: Record<O>, name: &str, v: f32, default: f32) -> Record<O> {
    or_default_with::<O, _>(rec, name, v.to_bits(), default.to_bits(), O::create_f32(v))
}
fn int_or_with<O: Ops>(rec: Record<O>, name: &str, v: i32, default: i32) -> Record<O> {
    or_default_with::<O, _>(rec, name, v, default, O::create_int(v))
}
fn bool_or_with<O: Ops>(rec: Record<O>, name: &str, v: bool, default: bool) -> Record<O> {
    or_default_with::<O, _>(rec, name, v, default, O::create_boolean(v))
}
fn optional_with<O: Ops, T>(
    r: &mut Reader,
    element: impl FnOnce(&mut Reader) -> Option<T>,
) -> Option<Option<T>> {
    if boolean_with::<O>(r)? {
        Some(Some(element(r)?))
    } else {
        Some(None)
    }
}
fn list_with<O: Ops, T>(
    r: &mut Reader,
    mut element: impl FnMut(&mut Reader) -> Option<T>,
) -> Option<Vec<T>> {
    let n = r.count(r.remaining()).ok()?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(element(r)?);
    }
    Some(out)
}

/// A `SoundEvent` holder: the codec form and, for a reference, its name (what a
/// field default compares against).
pub(super) struct Sound<O: Ops> {
    pub(super) code: O::Value,
    name: Option<String>,
}
/// `SoundEvent.STREAM_CODEC`: 0 and the event itself (`DIRECT_CODEC`: `sound_id`
/// and an optional `range`), or the registry id plus one.
pub(super) fn sound_with<O: Ops>(r: &mut Reader, names: &dyn Names) -> Option<Sound<O>> {
    match int_with::<O>(r)? {
        id if id < 0 => None,
        0 => {
            let id = ident_with::<O>(r)?;
            let range = optional_with::<O, _>(r, float_with::<O>)?;
            Some(Sound {
                code: Record::<O>::new()
                    .field("sound_id", id)
                    .optional("range", range.map(O::create_f32))
                    .build(),
                name: None,
            })
        }
        id => {
            let name = names.name("minecraft:sound_event", id - 1)?;
            Some(Sound {
                code: string_with::<O>(&name),
                name: Some(name),
            })
        }
    }
}
/// `SoundEvent.CODEC.optionalFieldOf(name, default)`.
fn sound_or_with<O: Ops>(rec: Record<O>, name: &str, s: Sound<O>, default: &str) -> Record<O> {
    if s.name.as_deref() == Some(default) {
        rec
    } else {
        rec.field(name, s.code)
    }
}
/// `ByteBufCodecs.holderSet`: 0 and a tag, or the size plus one and the ids.
/// `HolderSetCodec` writes `#tag`, a lone entry as itself, else a list.
fn holder_set_with<O: Ops>(registry: &str, names: &dyn Names, r: &mut Reader) -> Option<O::Value> {
    match int_with::<O>(r)? {
        size if size < 0 => None,
        0 => Some(string_with::<O>(&format!("#{}", r.string(32767).ok()?))),
        n => {
            let n = usize::try_from(n - 1)
                .ok()
                .filter(|n| *n <= r.remaining())?;
            let mut ids = Vec::with_capacity(n);
            for _ in 0..n {
                ids.push(entry_with::<O>(names, registry, int_with::<O>(r)?)?);
            }
            Some(if ids.len() == 1 {
                ids.remove(0)
            } else {
                O::create_list(ids)
            })
        }
    }
}
/// `ByteBufCodecs.holderRegistry`.
fn holder_with<O: Ops>(registry: &str, names: &dyn Names, r: &mut Reader) -> Option<O::Value> {
    entry_with::<O>(names, registry, int_with::<O>(r)?)
}

/// Structural failure rejects the component; a registered save refusal skips that effect.
fn consume_effects_with<O: Ops>(r: &mut Reader, names: &dyn Names) -> Option<Vec<O::Value>> {
    let n = r.count(r.remaining()).ok()?;
    let mut effects = Vec::with_capacity(n);
    for _ in 0..n {
        if let Some(effect) = consume_effect_with::<O>(r, names)? {
            effects.push(effect);
        }
    }
    Some(effects)
}

/// `ConsumeEffect.CODEC`: a `type` field and the type's fields.
fn consume_effect_with<O: Ops>(r: &mut Reader, names: &dyn Names) -> Option<Option<O::Value>> {
    use crate::item::consume_effects::{Kind, read_kind, read_native};
    let kind = read_kind(r).ok()?;
    let rec = Record::<O>::new();
    let value = match kind {
        Kind::Builtin(0) => {
            let effects = list_with::<O, _>(r, |r| super::raw::mob_effect_with::<O>(r, names))?;
            let probability = float_with::<O>(r)?;
            float_or_with::<O>(
                rec.field("type", string_with::<O>("minecraft:apply_effects"))
                    .field("effects", O::create_list(effects)),
                "probability",
                probability,
                1.0,
            )
        }
        Kind::Builtin(1) => rec
            .field("type", string_with::<O>("minecraft:remove_effects"))
            .field(
                "effects",
                holder_set_with::<O>("minecraft:mob_effect", names, r)?,
            ),
        Kind::Builtin(2) => rec.field("type", string_with::<O>("minecraft:clear_all_effects")),
        Kind::Builtin(3) => {
            let diameter = float_with::<O>(r)?;
            let particles = boolean_with::<O>(r)?;
            bool_or_with::<O>(
                float_or_with::<O>(
                    rec.field("type", string_with::<O>("minecraft:teleport_randomly")),
                    "diameter",
                    diameter,
                    16.0,
                ),
                "directional_particles",
                particles,
                true,
            )
        }
        Kind::Builtin(4) => rec
            .field("type", string_with::<O>("minecraft:play_sound"))
            .field("sound", sound_with::<O>(r, names)?.code),
        Kind::Builtin(_) => unreachable!("built-in consume-effect type"),
        Kind::Native(name, ty) => {
            return match read_native(name, ty, r).ok()? {
                Some(effect) => Some(Some(O::registered_consume_effect(effect)?)),
                None => Some(None),
            };
        }
    }
    .build();
    Some(Some(value))
}

/// `SwingAnimation`: `DEFAULT` is whack for 6 ticks.
fn swing_with<O: Ops>(r: &mut Reader) -> Option<O::Value> {
    let kind = int_with::<O>(r)?;
    let duration = int_with::<O>(r)?;
    let rec = or_default_with::<O, _>(
        Record::<O>::new(),
        "type",
        kind,
        1,
        pick_with::<O>(&SWING, kind)?,
    );
    Some(int_or_with::<O>(rec, "duration", duration, 6).build())
}

/// `ResolvableInt` / `ResolvableFloat`: a constant, or a context provider's key.
fn resolvable_with<O: Ops>(
    r: &mut Reader,
    constant: fn(&mut Reader) -> Option<O::Value>,
) -> Option<O::Value> {
    if boolean_with::<O>(r)? {
        constant(r)
    } else {
        ident_with::<O>(r)
    }
}
fn fixed_int_with<O: Ops>(r: &mut Reader) -> Option<O::Value> {
    Some(O::create_int(r.i32().ok()?))
}
fn fixed_float_with<O: Ops>(r: &mut Reader) -> Option<O::Value> {
    Some(O::create_f32(float_with::<O>(r)?))
}

/// `KineticWeapon.Condition`.
fn kinetic_condition_with<O: Ops>(r: &mut Reader) -> Option<O::Value> {
    let ticks = int_with::<O>(r)?;
    let speed = float_with::<O>(r)?;
    let relative = float_with::<O>(r)?;
    let rec = Record::<O>::new().field("max_duration_ticks", O::create_int(ticks));
    let rec = float_or_with::<O>(rec, "min_speed", speed, 0.0);
    Some(float_or_with::<O>(rec, "min_relative_speed", relative, 0.0).build())
}

/// The `Codec.BOOL` fields of the component predicates' codecs
/// (`FireworkPredicate`, `WrittenBookPredicate`, `MobEffectInstancePredicate`):
/// they travel as NBT bytes and hash as booleans. The text components inside a
/// written book's page predicates add [`TEXT_BOOLS`].
pub(super) const PREDICATE_BOOLS: [&str; 12] = [
    "has_twinkle",
    "has_trail",
    "resolved",
    "ambient",
    "visible",
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "interpret",
    "plain",
];
/// `DataComponentPredicate.STREAM_CODEC`: each entry is a type (a predicate
/// type, or a component type for `AnyValue`) and the predicate as the NBT its
/// `CODEC` encodes (`fromCodecWithRegistries`), hashed as the map
/// `DataComponentPredicate.CODEC` (`Codec.dispatchedMap`) writes.
fn partial_predicate_with<O: Ops>(
    r: &mut Reader,
    names: &dyn Names,
) -> Option<(O::Value, O::Value)> {
    let concrete = boolean_with::<O>(r)?;
    let registry = if concrete {
        "minecraft:data_component_predicate_type"
    } else {
        "minecraft:data_component_type"
    };
    let key = entry_with::<O>(names, registry, int_with::<O>(r)?)?;
    let tag = r.nbt().ok()?;
    if !concrete {
        return matches!(tag, sunrise_core::nbt::Tag::Compound(_)).then(|| (key, O::empty_map()));
    }
    // The nested `components` of an item predicate (`container`,
    // `bundle_contents`) are component values as NBT: `exact` hashes them by
    // each component's codec.
    Some((key, super::exact::predicate_with::<O>(&tag, names)?))
}

/// `BlockPredicate.STREAM_CODEC` as `BlockPredicate.CODEC` writes it.
fn block_predicate_with<O: Ops>(r: &mut Reader, names: &dyn Names) -> Option<O::Value> {
    let blocks = optional_with::<O, _>(r, |r| holder_set_with::<O>("minecraft:block", names, r))?;
    // StatePropertiesPredicate: a name and an exact value or a (min, max) range.
    let state = optional_with::<O, _>(r, |r| {
        let props = list_with::<O, _>(r, |r| {
            let name = string_with::<O>(&r.string(32767).ok()?);
            let matcher = if boolean_with::<O>(r)? {
                string_with::<O>(&r.string(32767).ok()?)
            } else {
                let min =
                    optional_with::<O, _>(r, |r| Some(string_with::<O>(&r.string(32767).ok()?)))?;
                let max =
                    optional_with::<O, _>(r, |r| Some(string_with::<O>(&r.string(32767).ok()?)))?;
                Record::<O>::new()
                    .optional("min", min)
                    .optional("max", max)
                    .build()
            };
            Some((name, matcher))
        })?;
        Some(O::create_map(props))
    })?;
    // NbtPredicate: `TagParser.LENIENT_CODEC` encodes with its flattened codec,
    // the tag's SNBT string.
    let nbt = optional_with::<O, _>(r, |r| match r.nbt().ok()? {
        tag @ sunrise_core::nbt::Tag::Compound(_) => Some(string_with::<O>(&to_snbt(&tag))),
        _ => None,
    })?;
    // DataComponentExactPredicate: the expected components, by type name.
    let exact = list_with::<O, _>(r, |r| {
        let _nested = crate::item::Nested::enter().ok()?;
        let kind = int_with::<O>(r)?;
        let name = entry_with::<O>(names, "minecraft:data_component_type", kind)?;
        let value = Component::read(kind, r).ok()?.code_with::<O>(kind, names)?;
        Some((name, value))
    })?;
    // DataComponentPredicate: the partial matchers (at most 64).
    let partial = list_with::<O, _>(r, |r| partial_predicate_with::<O>(r, names))?;
    if partial.len() > 64 {
        return None;
    }
    let rec = Record::<O>::new()
        .optional("blocks", blocks)
        .optional("state", state)
        .optional("nbt", nbt);
    let rec = if exact.is_empty() {
        rec
    } else {
        rec.field("components", O::create_map(exact))
    };
    Some(
        if partial.is_empty() {
            rec
        } else {
            rec.field("predicates", O::create_map(partial))
        }
        .build(),
    )
}

/// The hash of component `kind` from its stream, or `None`.
pub(super) fn code_with<O: Ops>(kind: i32, r: &mut Reader, names: &dyn Names) -> Option<O::Value> {
    Some(match kind {
        // UseEffects: canSprint, interactVibrations, speedMultiplier.
        5 => {
            let (sprint, vibrations, speed) = (
                boolean_with::<O>(r)?,
                boolean_with::<O>(r)?,
                float_with::<O>(r)?,
            );
            let rec = bool_or_with::<O>(Record::<O>::new(), "can_sprint", sprint, false);
            let rec = bool_or_with::<O>(rec, "interact_vibrations", vibrations, true);
            float_or_with::<O>(rec, "speed_multiplier", speed, 0.2).build()
        }
        // ItemAttributeModifiers: attribute, modifier, slot, display.
        16 => O::create_list(list_with::<O, _>(r, |r| {
            let attribute = holder_with::<O>("minecraft:attribute", names, r)?;
            let id = ident_with::<O>(r)?;
            let amount = r.f64().ok()?;
            let operation = pick_with::<O>(&ATTRIBUTE_OPERATIONS, int_with::<O>(r)?)?;
            let slot = int_with::<O>(r)?;
            let rec = Record::<O>::new()
                .field("type", attribute)
                .field("id", id)
                .field("amount", O::create_double(amount.to_bits()))
                .field("operation", operation);
            let rec = or_default_with::<O, _>(
                rec,
                "slot",
                slot,
                0,
                pick_with::<O>(&EQUIPMENT_SLOT_GROUPS, slot)?,
            );
            Some(
                match int_with::<O>(r)? {
                    0 => rec,
                    1 => rec.field(
                        "display",
                        Record::<O>::new()
                            .field("type", string_with::<O>("hidden"))
                            .build(),
                    ),
                    2 => rec.field(
                        "display",
                        Record::<O>::new()
                            .field("type", string_with::<O>("override"))
                            .field("value", text_with::<O>(&r.nbt().ok()?)?)
                            .build(),
                    ),
                    _ => return None,
                }
                .build(),
            )
        })?),
        // FoodProperties.DIRECT_CODEC: nutrition, saturation, canAlwaysEat.
        23 => {
            let nutrition = int_with::<O>(r)?;
            let saturation = float_with::<O>(r)?;
            bool_or_with::<O>(
                Record::<O>::new()
                    .field("nutrition", O::create_int(nutrition))
                    .field("saturation", O::create_f32(saturation)),
                "can_always_eat",
                boolean_with::<O>(r)?,
                false,
            )
            .build()
        }
        // Consumable: consumeSeconds, animation, sound, hasConsumeParticles, effects.
        24 => {
            let seconds = float_with::<O>(r)?;
            let animation = int_with::<O>(r)?;
            let snd = sound_with::<O>(r, names)?;
            let particles = boolean_with::<O>(r)?;
            let effects = consume_effects_with::<O>(r, names)?;
            let rec = float_or_with::<O>(Record::<O>::new(), "consume_seconds", seconds, 1.6);
            let rec = or_default_with::<O, _>(
                rec,
                "animation",
                animation,
                1,
                pick_with::<O>(&USE_ANIMATION, animation)?,
            );
            let rec = sound_or_with::<O>(rec, "sound", snd, "minecraft:entity.generic.eat");
            let rec = bool_or_with::<O>(rec, "has_consume_particles", particles, true);
            if effects.is_empty() {
                rec
            } else {
                rec.field("on_consume_effects", O::create_list(effects))
            }
            .build()
        }
        // UseCooldown: seconds, cooldownGroup.
        26 => {
            let seconds = float_with::<O>(r)?;
            Record::<O>::new()
                .field("seconds", O::create_f32(seconds))
                .optional("cooldown_group", optional_with::<O, _>(r, ident_with::<O>)?)
                .build()
        }
        // DamageResistant, Repairable: a holder set.
        27 => Record::<O>::new()
            .field(
                "types",
                holder_set_with::<O>("minecraft:damage_type", names, r)?,
            )
            .build(),
        33 => Record::<O>::new()
            .field("items", holder_set_with::<O>("minecraft:item", names, r)?)
            .build(),
        // Tool: rules, defaultMiningSpeed, damagePerBlock, canDestroyBlocksInCreative.
        28 => {
            let rules = list_with::<O, _>(r, |r| {
                let blocks = holder_set_with::<O>("minecraft:block", names, r)?;
                let speed = optional_with::<O, _>(r, float_with::<O>)?;
                let drops = optional_with::<O, _>(r, boolean_with::<O>)?;
                Some(
                    Record::<O>::new()
                        .field("blocks", blocks)
                        .optional("speed", speed.map(O::create_f32))
                        .optional("correct_for_drops", drops.map(O::create_boolean))
                        .build(),
                )
            })?;
            let (mining, damage, creative) = (
                float_with::<O>(r)?,
                int_with::<O>(r)?,
                boolean_with::<O>(r)?,
            );
            let rec = Record::<O>::new().field("rules", O::create_list(rules));
            let rec = float_or_with::<O>(rec, "default_mining_speed", mining, 1.0);
            let rec = int_or_with::<O>(rec, "damage_per_block", damage, 1);
            bool_or_with::<O>(rec, "can_destroy_blocks_in_creative", creative, true).build()
        }
        // Weapon: itemDamagePerAttack, disableBlockingForSeconds.
        29 => {
            let damage = int_with::<O>(r)?;
            let seconds = float_with::<O>(r)?;
            float_or_with::<O>(
                int_or_with::<O>(Record::<O>::new(), "item_damage_per_attack", damage, 1),
                "disable_blocking_for_seconds",
                seconds,
                0.0,
            )
            .build()
        }
        // Equippable.
        32 => {
            let slot = pick_with::<O>(&SLOT, int_with::<O>(r)?)?;
            let equip = sound_with::<O>(r, names)?;
            let asset = optional_with::<O, _>(r, ident_with::<O>)?;
            let overlay = optional_with::<O, _>(r, ident_with::<O>)?;
            let entities = optional_with::<O, _>(r, |r| {
                holder_set_with::<O>("minecraft:entity_type", names, r)
            })?;
            let flags = [
                boolean_with::<O>(r)?,
                boolean_with::<O>(r)?,
                boolean_with::<O>(r)?,
                boolean_with::<O>(r)?,
                boolean_with::<O>(r)?,
            ];
            let shearing = sound_with::<O>(r, names)?;
            let rec = Record::<O>::new().field("slot", slot);
            let rec = sound_or_with::<O>(
                rec,
                "equip_sound",
                equip,
                "minecraft:item.armor.equip_generic",
            )
            .optional("asset_id", asset)
            .optional("camera_overlay", overlay)
            .optional("allowed_entities", entities);
            let rec = bool_or_with::<O>(rec, "dispensable", flags[0], true);
            let rec = bool_or_with::<O>(rec, "swappable", flags[1], true);
            let rec = bool_or_with::<O>(rec, "damage_on_hurt", flags[2], true);
            let rec = bool_or_with::<O>(rec, "equip_on_interact", flags[3], false);
            let rec = bool_or_with::<O>(rec, "can_be_sheared", flags[4], false);
            sound_or_with::<O>(
                rec,
                "shearing_sound",
                shearing,
                "minecraft:item.shears.snip",
            )
            .build()
        }
        // DeathProtection: deathEffects.
        36 => {
            let effects = consume_effects_with::<O>(r, names)?;
            if effects.is_empty() {
                Record::<O>::new()
            } else {
                Record::<O>::new().field("death_effects", O::create_list(effects))
            }
            .build()
        }
        // BlocksAttacks.
        37 => {
            let delay = float_with::<O>(r)?;
            let scale = float_with::<O>(r)?;
            let reductions = list_with::<O, _>(r, |r| {
                let angle = float_with::<O>(r)?;
                let types = optional_with::<O, _>(r, |r| {
                    holder_set_with::<O>("minecraft:damage_type", names, r)
                })?;
                let (base, factor) = (float_with::<O>(r)?, float_with::<O>(r)?);
                Some((angle, types, base, factor))
            })?;
            let item = [
                float_with::<O>(r)?,
                float_with::<O>(r)?,
                float_with::<O>(r)?,
            ];
            let bypassed = optional_with::<O, _>(r, |r| {
                holder_set_with::<O>("minecraft:damage_type", names, r)
            })?;
            let block = optional_with::<O, _>(r, |r| sound_with::<O>(r, names))?;
            let disabled = optional_with::<O, _>(r, |r| sound_with::<O>(r, names))?;
            let rec = float_or_with::<O>(Record::<O>::new(), "block_delay_seconds", delay, 0.0);
            let rec = float_or_with::<O>(rec, "disable_cooldown_scale", scale, 1.0);
            // The default is one reduction: 90 degrees, any damage, base 0, factor 1.
            let is_default = matches!(
                &reductions[..],
                [(a, None, b, f)] if a.to_bits() == 90f32.to_bits()
                    && b.to_bits() == 0f32.to_bits() && f.to_bits() == 1f32.to_bits()
            );
            let rec = if is_default {
                rec
            } else {
                rec.field(
                    "damage_reductions",
                    O::create_list(reductions.into_iter().map(|(angle, types, base, factor)| {
                        float_or_with::<O>(
                            Record::<O>::new(),
                            "horizontal_blocking_angle",
                            angle,
                            90.0,
                        )
                        .optional("type", types)
                        .field("base", O::create_f32(base))
                        .field("factor", O::create_f32(factor))
                        .build()
                    })),
                )
            };
            let rec = if item.map(f32::to_bits) == [1.0f32, 0.0, 1.0].map(f32::to_bits) {
                rec
            } else {
                rec.field(
                    "item_damage",
                    Record::<O>::new()
                        .field("threshold", O::create_f32(item[0]))
                        .field("base", O::create_f32(item[1]))
                        .field("factor", O::create_f32(item[2]))
                        .build(),
                )
            };
            rec.optional("bypassed_by", bypassed)
                .optional("block_sound", block.map(|s| s.code))
                .optional("disabled_sound", disabled.map(|s| s.code))
                .build()
        }
        // PiercingWeapon: dealsKnockback, dismounts, sound, hitSound.
        38 => {
            let (knockback, dismounts) = (boolean_with::<O>(r)?, boolean_with::<O>(r)?);
            let snd = optional_with::<O, _>(r, |r| sound_with::<O>(r, names))?;
            let hit = optional_with::<O, _>(r, |r| sound_with::<O>(r, names))?;
            bool_or_with::<O>(
                bool_or_with::<O>(Record::<O>::new(), "deals_knockback", knockback, true),
                "dismounts",
                dismounts,
                false,
            )
            .optional("sound", snd.map(|s| s.code))
            .optional("hit_sound", hit.map(|s| s.code))
            .build()
        }
        // KineticWeapon.
        39 => {
            let (cooldown, delay) = (int_with::<O>(r)?, int_with::<O>(r)?);
            let dismount = optional_with::<O, _>(r, kinetic_condition_with::<O>)?;
            let knockback = optional_with::<O, _>(r, kinetic_condition_with::<O>)?;
            let damage = optional_with::<O, _>(r, kinetic_condition_with::<O>)?;
            let (forward, multiplier) = (float_with::<O>(r)?, float_with::<O>(r)?);
            let snd = optional_with::<O, _>(r, |r| sound_with::<O>(r, names))?;
            let hit = optional_with::<O, _>(r, |r| sound_with::<O>(r, names))?;
            let rec = int_or_with::<O>(Record::<O>::new(), "contact_cooldown_ticks", cooldown, 10);
            let rec = int_or_with::<O>(rec, "delay_ticks", delay, 0)
                .optional("dismount_conditions", dismount)
                .optional("knockback_conditions", knockback)
                .optional("damage_conditions", damage);
            let rec = float_or_with::<O>(rec, "forward_movement", forward, 0.0);
            float_or_with::<O>(rec, "damage_multiplier", multiplier, 1.0)
                .optional("sound", snd.map(|s| s.code))
                .optional("hit_sound", hit.map(|s| s.code))
                .build()
        }
        40 | 41 => swing_with::<O>(r)?,
        // `JukeboxSong.CODEC` is a `RegistryFixedCodec`: a direct song cannot be
        // encoded (vanilla's client would throw), so only a reference hashes.
        66 => match int_with::<O>(r)? {
            id if id < 0 => return None,
            0 => O::stream_only(
                Record::<O>::new()
                    .field("sound_event", sound_with::<O>(r, names)?.code)
                    .field("description", text_with::<O>(&r.nbt().ok()?)?)
                    .field("length_in_seconds", O::create_f32(float_with::<O>(r)?))
                    .field("comparator_output", O::create_int(int_with::<O>(r)?))
                    .build(),
            )?,
            id => entry_with::<O>(names, "minecraft:jukebox_song", id - 1)?,
        },
        67 => holder_set_with::<O>("minecraft:banner_pattern", names, r)?,
        83 => sound_with::<O>(r, names)?.code,
        84 => Record::<O>::new()
            .field("layers", resolvable_with::<O>(r, fixed_int_with::<O>)?)
            .build(),
        // CookingFuel (burnTime, speedMultiplier), BrewingFuel (uses, speedMultiplier).
        85 | 86 => {
            let first = resolvable_with::<O>(r, fixed_int_with::<O>)?;
            let speed = resolvable_with::<O>(r, fixed_float_with::<O>)?;
            Record::<O>::new()
                .field(if kind == 85 { "burn_time" } else { "uses" }, first)
                .field("speed_multiplier", speed)
                .build()
        }
        // MobVisibility: targetingEntityTypes, visibility.
        87 => {
            let types = holder_set_with::<O>("minecraft:entity_type", names, r)?;
            Record::<O>::new()
                .field("targeting_entity_types", types)
                .field("visibility", O::create_f32(float_with::<O>(r)?))
                .build()
        }
        // AdventureModePredicate (can_place_on, can_break): `compactListCodec`
        // writes one predicate as itself and `nonEmptyList` refuses none.
        14 | 15 => {
            let mut predicates = list_with::<O, _>(r, |r| block_predicate_with::<O>(r, names))?;
            match predicates.len() {
                0 => return None,
                1 => predicates.remove(0),
                _ => O::create_list(predicates),
            }
        }
        _ => return None,
    })
}
