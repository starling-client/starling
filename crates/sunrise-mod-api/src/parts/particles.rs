//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl ParticlesRegistrar for PartRegistrar {
    fn particle_key(&mut self, id: &str) -> crate::ParticleKey {
        match self
            .registry_keys
            .borrow_mut()
            .particles
            .request(self.mod_id, id)
        {
            Ok(slot) => mod_vocab::__private::parts_installation::particle_key(slot),
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::particle_key(u32::MAX)
            }
        }
    }

    fn add_particle_type(
        &mut self,
        id: &str,
        override_limiter: bool,
        codec: Arc<dyn mod_vocab::__private::ParticleCodec>,
    ) -> crate::ParticleKey {
        let result = (|| {
            let id = self.declaration_id(id)?;
            let packages = crate::load_state::current_packages();
            let namespace = id.split_once(':').map_or("", |(namespace, _)| namespace);
            if let Some(package) = packages.iter().find(|p| {
                p.side() != crate::Side::Both
                    && (p.id() == self.mod_id
                        || (p.served_by() == Some(self.mod_id)
                            && p.namespaces().any(|owned| owned == namespace)))
            }) {
                return Err(format!(
                    "mod {} particle type {id}: package {} must use side = \"both\" for a networked particle type",
                    self.mod_id,
                    package.id()
                ));
            }
            let slot = self
                .registry_keys
                .borrow_mut()
                .particles
                .request(self.mod_id, &id)?;
            let key = mod_vocab::__private::parts_installation::particle_key(slot);
            match self.parts.particle_types.entry(key) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert((
                        override_limiter,
                        mod_vocab::__private::own_particle_codec(
                            self.mod_id,
                            self.mod_index,
                            codec,
                        ),
                    ));
                    Ok(key)
                }
                std::collections::hash_map::Entry::Occupied(_) => Err(format!(
                    "mod {} registered particle type {id} twice",
                    self.mod_id
                )),
            }
        })();
        match result {
            Ok(key) => key,
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::particle_key(u32::MAX)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hooks, ModEntry, RegistrationContext};
    use std::sync::Mutex;
    #[test]
    fn particle_registration_retains_requested_slots_until_atomic_publication() {
        struct Keys(Arc<Mutex<Vec<crate::ParticleKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Keys {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                let custom = registrar.particle_key("test:spark");
                let smoke = registrar.particle_key("smoke");
                assert_eq!(
                    mod_vocab::__private::parts_installation::validate_particle_key(
                        mod_vocab::__private::parts_installation::particle_key_slot(custom)
                    ),
                    Ok(custom)
                );
                assert_eq!(smoke, registrar.particle_key("minecraft:smoke"));
                self.0.lock().unwrap().extend([custom, smoke]);
            }
        }
        let keys = Arc::new(Mutex::new(Vec::new()));
        install_parts(
            &[ModEntry::new("keys", Box::new(Keys(keys.clone())))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        let keys = keys.lock().unwrap();
        assert!(
            mod_vocab::__private::resolve_particles(["minecraft:smoke"], |_| Some(
                sunrise_mod_runtime::registry_keys::ParticleKind::Custom
            ))
            .is_err()
        );
        assert!(!mod_vocab::__private::particles_resolved());
        mod_vocab::__private::resolve_particles(["minecraft:smoke", "test:spark"], |id| {
            Some(if id == "minecraft:smoke" {
                sunrise_mod_runtime::registry_keys::ParticleKind::Vanilla(5)
            } else {
                sunrise_mod_runtime::registry_keys::ParticleKind::Custom
            })
        })
        .unwrap();
        assert_eq!(
            mod_vocab::__private::particle_name(keys[0]),
            Some("test:spark")
        );
        assert_eq!(mod_vocab::__private::particle_vanilla_key(5), Some(keys[1]));
        assert_eq!(
            mod_vocab::__private::particle_kind(keys[0]),
            Some(sunrise_mod_runtime::registry_keys::ParticleKind::Custom)
        );
    }

    fn map<const SHIFT: i32>() -> crate::ParticleMapCodec<i32> {
        crate::ParticleMapCodec::new(
            |fields, _| match fields.get(&"value".into()) {
                Some(crate::Tag::Int(value)) => {
                    value.checked_add(SHIFT).ok_or("value overflow".into())
                }
                _ => Err("value is required".into()),
            },
            |value, _| {
                Ok(crate::Compound::from([(
                    "value".into(),
                    crate::Tag::Int(value - SHIFT),
                )]))
            },
        )
    }

    struct Codecs {
        duplicate: bool,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Codecs {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            let left = registrar.register_particle_type(
                "left",
                true,
                map::<0>(),
                crate::ParticleStreamCodec::wire(),
            );
            assert_eq!(left.key(), registrar.particle_key("test:left"));
            registrar.register_particle_type(
                if self.duplicate { "left" } else { "right" },
                false,
                map::<1>(),
                crate::ParticleStreamCodec::mapped(
                    |value: i16, _| Ok(i32::from(value)),
                    |value, _| i16::try_from(*value).map_err(|e| e.to_string()),
                ),
            );
        }
    }

    #[test]
    fn common_particle_types_keep_their_distinct_codecs_without_assets() {
        let entries = [ModEntry::new("test", Box::new(Codecs { duplicate: false }))];
        crate::parts::tests::install_parts_fixture(&entries).unwrap();
        mod_vocab::__private::resolve_particles([], |_| {
            Some(mod_vocab::__private::ParticleKind::Custom)
        })
        .unwrap();
        let fields = crate::Compound::from([("value".into(), crate::Tag::Int(17))]);
        for (name, override_limiter, bytes) in [
            ("test:left", true, &[17, 0, 0, 0][..]),
            ("test:right", false, &[18, 0][..]),
        ] {
            let value = mod_vocab::__private::decode_particle_map(name, &fields)
                .unwrap()
                .unwrap();
            assert_eq!(
                mod_vocab::__private::particle_override_limiter(value.key()),
                Some(override_limiter)
            );
            assert_eq!(
                mod_vocab::__private::encode_particle_stream(&value, 256).unwrap(),
                bytes
            );
            assert_eq!(
                mod_vocab::__private::decode_particle_stream(name, bytes).unwrap(),
                Some(value)
            );
        }
    }

    #[test]
    fn duplicate_particle_type_refuses_atomic_installation() {
        let entries = [ModEntry::new("test", Box::new(Codecs { duplicate: true }))];
        let errors = crate::parts::tests::install_parts_fixture(&entries).unwrap_err();
        let [error] = errors.as_slice() else {
            panic!("expected one refusal: {errors:?}")
        };
        assert!(error.contains("mod test") && error.contains("test:left"));
        assert!(!mod_vocab::__private::parts_installation::parts_installed());
        assert!(mod_vocab::__private::particle_named_key("test:left").is_none());
    }
}
