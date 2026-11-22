//! Registration of the persistent and stream codecs for a custom consume effect.
use crate::{Compound, Identifier, runtime::invoke_registered_codec, wire};
use std::sync::{Arc, atomic::AtomicBool};

// no wire: this is a process-local codec registration; only its values have a Wire form.
/// A consume-effect type's save codec and its typed `Wire` stream form.
#[non_exhaustive]
pub struct ConsumeEffectType {
    install: Box<InstallCodec>,
}

type InstallCodec = dyn Fn(&'static str) -> ConsumeEffectCodecs + Send + Sync;
type Load = dyn Fn(&Compound) -> Result<(Compound, Vec<u8>), String> + Send + Sync;
type Read = dyn Fn(&[u8], bool) -> Result<(usize, Option<Compound>), String> + Send + Sync;

// no wire: type-erased engine adapter for the map and Wire codec callbacks.
// hidden: the engine installs type-erased callbacks; mods register a ConsumeEffectType.
#[non_exhaustive]
#[doc(hidden)]
pub struct ConsumeEffectCodecs {
    pub schema: u64,
    pub load: Box<Load>,
    pub read: Box<Read>,
}

impl ConsumeEffectType {
    /// Construct a type from its persistent map codec. Implement the stream
    /// form by deriving [`Wire`](crate::Wire) on `T`. The game handles the
    /// dispatch key `type`; neither function reads or writes that field.
    /// `load` validates fields in decoded `minecraft:consumable` and
    /// `minecraft:death_protection` overrides. Use these full keys in raw stack
    /// patches: bare keys are not admitted as overrides of these components.
    /// Effects in a `minecraft:use_remainder` or `minecraft:sulfur_cube_content`
    /// template are decoded when the template is materialized, not when the
    /// parent is written. `save` writes the canonical fields when a value is
    /// admitted, including when it arrives over Wire.
    /// Neither callback depends on the live gameplay listener being enabled.
    /// Both can run on load and network threads. Return an error for invalid
    /// data rather than panicking: an error refuses that value, whereas a panic
    /// disables that callback for the session and names the mod in the log and
    /// error toast. Reopening a save retains an effect's original data if its
    /// codec fails, without running the effect. Failure in a map callback does
    /// not disable structural Wire decoding.
    pub fn new<T: wire::Wire + 'static>(
        load: fn(&Compound) -> Result<T, String>,
        save: fn(&T) -> Result<Compound, String>,
    ) -> Self {
        let wire_schema = T::describe(&mut wire::WireSchema::default());
        Self {
            install: Box::new(move |mod_id| {
                let load_panicked = AtomicBool::new(false);
                let write_panicked = AtomicBool::new(false);
                let read_panicked = AtomicBool::new(false);
                let save_panicked = Arc::new(AtomicBool::new(false));
                let save_value = move |value: &T| {
                    invoke_registered_codec(mod_id, "consume_effect_save", &save_panicked, || {
                        save(value)
                    })
                };
                let save_for_load = save_value.clone();
                ConsumeEffectCodecs {
                    schema: wire_schema,
                    load: Box::new(move |fields| {
                        invoke_registered_codec(
                            mod_id,
                            "consume_effect_load",
                            &load_panicked,
                            || {
                                let value = load(fields)?;
                                let mut out = wire::Writer::new();
                                let encoded = invoke_registered_codec(
                                    mod_id,
                                    "consume_effect_wire_write",
                                    &write_panicked,
                                    || {
                                        out.put(&value);
                                        out.error().map_or(Ok(()), |error| Err(error.to_string()))
                                    },
                                );
                                // Retain the typed value inside the invocation fence,
                                // but don't report an already-attributed error twice.
                                Ok(encoded
                                    .and_then(|()| Ok((save_for_load(&value)?, out.into_bytes()))))
                            },
                        )?
                    }),
                    read: Box::new(move |bytes, project| {
                        invoke_registered_codec(
                            mod_id,
                            "consume_effect_wire_read",
                            &read_panicked,
                            || {
                                let mut reader = wire::Reader::new(bytes);
                                let value =
                                    reader.take::<T>().map_err(|error| error.to_string())?;
                                let saved = if project {
                                    save_value(&value).ok()
                                } else {
                                    None
                                };
                                Ok((bytes.len() - reader.rest().len(), saved))
                            },
                        )
                    }),
                }
            }),
        }
    }
}

/// Consume-effect type registration.
pub trait ConsumeEffectsRegistrar {
    /// Register a custom kind during `Hooks::register`. The registry and stream
    /// schema are synchronized before play. Both peers install the same type;
    /// the identifier's namespace must be the registering package's ID, which
    /// is also the runtime dispatch owner.
    /// Duplicate IDs and server-only packages are refused.
    fn register_consume_effect_type(
        &mut self,
        id: Identifier,
        ty: ConsumeEffectType,
    ) -> Result<(), String>;
}

// no wire: engine-owned collection entry with process-local callbacks.
// exhaustive: its private owner and codecs are supplied by the Parts registrar.
/// An attributed consume-effect registration, not exposed to mods.
// hidden: the Parts registrar attaches the runtime owner after validating the declaration.
#[doc(hidden)]
pub struct RegisteredConsumeEffect {
    owner: &'static str,
    /// Canonical dispatch key, read by the registrar to reject duplicates.
    pub id: Identifier,
    ty: ConsumeEffectType,
}

impl RegisteredConsumeEffect {
    /// Form the registry row for an already validated and attributed declaration.
    pub fn new(owner: &'static str, id: Identifier, ty: ConsumeEffectType) -> Self {
        Self { owner, id, ty }
    }
}

/// Take the registered engine-side codecs after mod registration has finished.
// hidden: the engine builds the codec table from the frozen Parts, not from mod code.
#[doc(hidden)]
pub fn consume_effect_codecs() -> Vec<(String, ConsumeEffectCodecs)> {
    super::installed()
        .into_iter()
        .flat_map(|parts| &parts.consume_effects)
        .map(|effect| (effect.id.to_string(), (effect.ty.install)(effect.owner)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_panic_uses_shared_diagnostics_without_poisoning_wire() {
        crate::testing::with_dispatch(Default::default(), || {
            let ty = ConsumeEffectType::new::<u16>(
                |_| Ok(0x1234),
                |value| {
                    assert_ne!(*value, 0x1234, "fixture save panic");
                    Ok(Compound::new())
                },
            );
            let codec = (ty.install)("effect_test");
            // The first value panics only in the map projection; framing is still known.
            assert_eq!((codec.read)(&[0x34, 0x12, 0xff], true), Ok((2, None)));
            // The next value is valid and would save successfully if the callback ran.
            assert_eq!((codec.read)(&[0x78, 0x56, 0xff], true), Ok((2, None)));
            assert_eq!((codec.read)(&[0x78, 0x56, 0xff], false), Ok((2, None)));
            assert_eq!(
                crate::__private::take_hook_panics(),
                [crate::__private::HookPanicReport {
                    mod_id: "effect_test",
                    hook: "consume_effect_save",
                }],
            );
        });
    }
}
