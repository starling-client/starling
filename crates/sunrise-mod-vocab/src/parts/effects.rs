use super::*;

type EffectTicks = HashMap<Box<str>, Vec<Attached<dyn MobEffectPart>>>;

// no wire: the load-owned table contains process-local trait objects.
// hidden: opaque resolution slot carried by the installation record.
/// Parts subscribed to effect ticks, frozen after effect keys and owners resolve.
#[doc(hidden)]
#[derive(Default)]
#[non_exhaustive]
pub struct ResolvedEffectTicks(OnceLock<EffectTicks>);

pub(crate) fn resolved_effects() -> &'static OnceLock<sunrise_mod_runtime::registry_keys::Effects> {
    static EMPTY: OnceLock<sunrise_mod_runtime::registry_keys::Effects> = OnceLock::new();
    installed().map_or(&EMPTY, |installed| &installed.effects)
}

/// Registrar operations for effects.
pub trait EffectsRegistrar {
    /// Request an effect by canonical registry id. All mods' declarations are checked at load.
    fn effect_key(&mut self, id: &str) -> crate::EffectKey;
    /// Attach tick behaviour to an effect declared by this package, by a content package it
    /// serves (`served_by`), or by an active dependency.
    /// An absent package with an active optional dependency row skips its Parts; other unknown
    /// effects refuse loading.
    fn attach_mob_effect_part(
        &mut self,
        effect: &crate::EffectKey,
        kind: &'static str,
        part: Arc<dyn MobEffectPart>,
    ) -> Result<(), MobEffectPartError>;
}

/// A refused effect Part registration.
///
/// This is a process-local registration error, not a `Wire` type.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::MobEffectPartError>();
/// ```
// no wire: local mob-effect Part registration error.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MobEffectPartError {
    /// Vanilla effects keep vanilla tick behaviour.
    VanillaEffect {
        /// The vanilla effect that was requested.
        effect: crate::EffectKey,
    },
    /// This concrete Part type is already attached to the effect.
    Duplicate {
        /// The effect already carrying this Part type.
        effect: crate::EffectKey,
        /// The concrete Rust Part type name.
        part: String,
    },
    /// The effect key does not resolve to a requested registry slot.
    InvalidKey {
        /// The unresolved effect key.
        effect: crate::EffectKey,
    },
}

impl dyn crate::Registrar + '_ {
    /// Attach a Part to an effect declared by this package, by a content package it serves
    /// (`served_by`), or by an active dependency.
    /// The owning package's effect Parts (or, for a `served_by` content package, its serving
    /// native host's) run first. Other Parts execute in
    /// dispatch order, then registration order. Each Part keeps its own cadence.
    /// Vanilla effects refuse immediately.
    pub fn mob_effect_part<P: MobEffectPart>(
        &mut self,
        effect: &crate::EffectKey,
        part: P,
    ) -> Result<(), MobEffectPartError> {
        self.attach_mob_effect_part(effect, type_name::<P>(), Arc::new(part))
    }
}

/// Validate Part permissions against the actual effect owners and freeze tick order.
/// Call after effect-key completion, so unknown effects retain their ordinary refusal.
pub fn validate_effect_part_owners<'a>(
    owner: impl Fn(&str) -> Option<&'a str>,
) -> Result<(), Vec<String>> {
    let Some(installed) = installed() else {
        return Ok(());
    };
    if installed.effect_ticks.0.get().is_some() {
        return Ok(());
    }
    let mut ids: Vec<_> = installed.keys.effects.keys().collect();
    ids.sort_unstable();
    let mut refused = Vec::new();
    let mut ticks = EffectTicks::new();
    let subscribed = installed
        .keys
        .for_hook(HookId::ShouldApplyEffectTickThisTick);
    for id in ids {
        let parts = &installed.keys.effects[id];
        if parts.is_empty() {
            continue;
        }
        let Some(owner) = owner(id) else {
            for part in parts {
                refused.push(format!("mod {} requested unknown effect {id}", part.mod_id));
            }
            continue;
        };
        for part in parts {
            if !installed.dependencies.permits(part.mod_id, owner) {
                refused.push(format!("mod {} attached {} to effect {id}, declared by package {owner}; declare {owner} as a dependency", part.mod_id, part.type_name));
            }
        }
        if let Some(parts) = subscribed.and_then(|table| table.effects.get(id)) {
            let mut parts = parts.clone();
            parts.sort_by_key(|part| (part.mod_id != owner, part.mod_index, part.order));
            ticks.insert(id.clone(), parts);
        }
    }
    if refused.is_empty() {
        let _ = installed.effect_ticks.0.set(ticks);
        Ok(())
    } else {
        Err(refused)
    }
}

/// Run each effect Part's cadence and action as one pair, sharing the holder snapshots.
/// Queued writes remain deferred until this entire dispatch returns.
pub fn dispatch_mob_effect_tick<'a>(
    host: &dyn Host,
    tick_count: i32,
    effect: crate::EffectKey,
    amplifier: i32,
    make_call: impl FnOnce(crate::EffectKey, i32) -> crate::__private::ApplyEffectTickCall<'a>,
) {
    let Some(ticks) = installed().and_then(|installed| installed.effect_ticks.0.get()) else {
        return;
    };
    let Some(attached) =
        crate::content::registry_keys::effect_id(effect).and_then(|id| ticks.get(id))
    else {
        return;
    };
    let mut due = crate::entered_parts(
        HookId::ShouldApplyEffectTickThisTick,
        attached,
        "should_apply_effect_tick_this_tick",
    )
    .filter(|&(mod_id, part)| {
        let mut should = crate::ShouldApplyEffectTickThisTickEvent {
            effect,
            tick_count,
            amplifier,
            apply: false,
        };
        crate::runtime::invoke(
            HookId::ShouldApplyEffectTickThisTick,
            "should_apply_effect_tick_this_tick",
            mod_id,
            || {
                part.should_apply_effect_tick_this_tick(host, &mut should);
                should.apply
            },
        )
        .unwrap_or_default()
    });
    let Some(first) = due.next() else {
        return;
    };
    let call = make_call(effect, amplifier);
    for (mod_id, part) in std::iter::once(first).chain(due) {
        if crate::runtime::poisoned(HookId::ApplyEffectTick, mod_id) {
            continue;
        }
        let changes_len = call.changes.len();
        match crate::runtime::invoke(HookId::ApplyEffectTick, "apply_effect_tick", mod_id, || {
            let mut event = crate::ApplyEffectTickEvent {
                effect: call.effect,
                dimension: call.dimension,
                entity: call.entity,
                amplifier: call.amplifier,
                holder: call.holder,
                health: call.health,
                max_health: call.max_health,
                food: call.food,
                active_effects: call.active_effects,
                changes: &mut *call.changes,
                keep: *call.keep,
            };
            part.apply_effect_tick(host, &mut event);
            event.keep
        }) {
            Some(keep) => {
                *call.keep = keep;
                if !*call.keep {
                    break;
                }
            }
            None => {
                call.changes.truncate(changes_len);
            }
        }
    }
}
