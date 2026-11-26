//! Concrete authoring extracted from the vocabulary layer.
use super::*;

impl GeneralTypesRegistrar for PartRegistrar {
    fn declare_general_entity(
        &mut self,
        path: &str,
        category: GeneralEntityCategory,
        width: f64,
        height: f64,
        passenger_attachment: Option<[f64; 3]>,
        save_policy: GeneralSavePolicy,
        summonable: bool,
        pickable: bool,
        fire_immune: bool,
        piston_push_reaction: Option<PushReaction>,
        tracking_chunks: i32,
        update_interval: i32,
        payload_version: u16,
        required_features: &[crate::FeatureFlag],
    ) -> Result<crate::EntityTypeKey, GeneralEntityRegistrationError> {
        use GeneralEntityRegistrationError as Error;
        // The registry freezes once startup registration closes: no declaration is accepted
        // outside it.
        if !crate::registration_open() {
            self.refused.push(format!(
                "mod {} declares a general entity type after the registry froze",
                self.mod_id
            ));
            return Err(Error::Frozen);
        }
        let id = match self.declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return Err(Error::InvalidId);
            }
        };
        let id_for_log = id.clone();
        let result = (|| {
            if self.declaration_side_not_both(&id) {
                return Err(Error::WrongSide);
            }
            if self
                .general_entity_types
                .iter()
                .any(|declared| declared.id == id)
                || self.has_builtin_entry("minecraft:entity_type", &id)
            {
                return Err(Error::Duplicate);
            }
            if ![width, height]
                .into_iter()
                .chain(passenger_attachment.into_iter().flatten())
                .all(f64::is_finite)
            {
                return Err(Error::NotFinite);
            }
            let mut features = Vec::with_capacity(required_features.len());
            for &flag in required_features {
                if !features.contains(&flag) {
                    features.push(flag);
                }
            }
            let key = self.entity_type_key(&id);
            self.general_entity_types
                .push(GeneralEntityTypeDeclaration {
                    mod_id: self.mod_id.to_owned(),
                    id,
                    category,
                    width,
                    height,
                    passenger_attachment,
                    save_policy,
                    summonable,
                    pickable,
                    fire_immune,
                    piston_push_reaction: piston_push_reaction.unwrap_or_default(),
                    tracking_chunks,
                    update_interval,
                    payload_version,
                    required_features: features,
                });
            Ok(key)
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} refuses general entity type {id_for_log:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::__private::RegistrationGuard;
    use crate::GeneralEntityRegistrationError as Error;

    fn registrar() -> (RegistrationGuard, PartRegistrar) {
        // Declarations are accepted only while startup registration is open.
        let guard = RegistrationGuard::open();
        (guard, PartRegistrar::new("mymod"))
    }

    fn declare(
        registrar: &mut PartRegistrar,
        path: &str,
        width: f64,
        height: f64,
        tracking_chunks: i32,
        update_interval: i32,
        required: &[crate::FeatureFlag],
    ) -> Result<crate::EntityTypeKey, Error> {
        registrar.declare_general_entity(
            path,
            GeneralEntityCategory::Misc,
            width,
            height,
            None,
            GeneralSavePolicy::Save,
            true,
            false,
            false,
            None,
            tracking_chunks,
            update_interval,
            1,
            required,
        )
    }

    /// Naming, built-in reservations and canonical handles share one registry; feature flags
    /// keep their vanilla meaning and valid long identifiers have no synthetic cap.
    #[test]
    pub(crate) fn general_entity_type_declarations_validate() {
        crate::__private::initialize().unwrap();
        let data = mod_vocab::__private::RegistrationData::new(
            [(
                "minecraft:entity_type".into(),
                vec!["minecraft:creeper".into()],
            )]
            .into(),
            Default::default(),
            crate::InstanceDirectory::from_path(std::path::Path::new("unused")).unwrap(),
            crate::RuntimeEnvironment::new(
                crate::Dist::Client,
                "26.3".into(),
                "test".into(),
                false,
            ),
        );
        crate::load_state::with_test_registration_data(data, || {
            let (_phase, mut registrar) = registrar();
            let robot = declare(&mut registrar, "robot", 0.8, 1.9, 8, 3, &[]).unwrap();
            assert_eq!(robot, registrar.entity_type_key("mymod:robot"));
            // Explicit namespaces are independent of the declaring package.
            assert_eq!(
                declare(&mut registrar, "minecraft:creeper", 0.6, 1.7, 8, 3, &[]),
                Err(Error::Duplicate)
            );
            for id in ["foreign:robot", "minecraft:custom_robot"] {
                let key = declare(&mut registrar, id, 0.6, 1.7, 8, 3, &[]).unwrap();
                assert_eq!(key, registrar.entity_type_key(id));
            }
            assert_eq!(
                declare(&mut registrar, "Not An Id", 0.6, 1.7, 8, 3, &[]),
                Err(Error::InvalidId)
            );
            // No synthetic 256-byte cap at registration: valid long identifiers are accepted.
            let long_path = "l".repeat(300);
            let long_key = declare(&mut registrar, &long_path, 0.6, 1.7, 8, 3, &[]).unwrap();
            assert_eq!(
                long_key,
                registrar.entity_type_key(&format!("mymod:{long_path}"))
            );
            // The same type twice, whatever declared it, is a duplicate.
            assert_eq!(
                declare(&mut registrar, "robot", 0.5, 1.0, 4, 2, &[]),
                Err(Error::Duplicate)
            );
            // Non-finite dimensions.
            assert_eq!(
                declare(&mut registrar, "wide", f64::NAN, 1.0, 8, 3, &[]),
                Err(Error::NotFinite)
            );
            assert_eq!(
                declare(&mut registrar, "wide", f64::INFINITY, 1.0, 8, 3, &[]),
                Err(Error::NotFinite)
            );
            // Real Minecraft feature flags are accepted without treating them as mod dependencies.
            let flagged = declare(
                &mut registrar,
                "experimental",
                1.0,
                1.0,
                8,
                3,
                &[
                    crate::FeatureFlag::RedstoneExperiments,
                    crate::FeatureFlag::MinecartImprovements,
                    crate::FeatureFlag::RedstoneExperiments,
                ],
            )
            .unwrap();
            assert_eq!(flagged, registrar.entity_type_key("mymod:experimental"));
            // Every refusal was reported, and the good declarations stand.
            assert_eq!(registrar.general_entity_types.len(), 5);
            for (entry, id) in registrar.general_entity_types.iter().zip([
                "mymod:robot",
                "foreign:robot",
                "minecraft:custom_robot",
            ]) {
                assert_eq!(entry.id, id);
                assert_eq!(entry.mod_id, "mymod");
            }
            assert_eq!(
                registrar.general_entity_types[4].required_features,
                vec![
                    crate::FeatureFlag::RedstoneExperiments,
                    crate::FeatureFlag::MinecartImprovements,
                ]
            );
            assert_eq!(registrar.refused.len(), 5);
        });
    }

    #[test]
    fn a_non_finite_passenger_attachment_refuses_registration() {
        crate::__private::initialize().unwrap();
        let (_phase, mut registrar) = registrar();
        assert_eq!(
            registrar.declare_general_entity(
                "robot",
                GeneralEntityCategory::Misc,
                0.8,
                1.9,
                Some([1.0, f64::NAN, 0.0]),
                GeneralSavePolicy::Save,
                true,
                false,
                false,
                None,
                8,
                3,
                1,
                &[],
            ),
            Err(Error::NotFinite)
        );
        assert!(registrar.general_entity_types.is_empty());
    }

    #[test]
    fn vanilla_ranges_and_no_update_interval_are_accepted() {
        crate::__private::initialize().unwrap();
        let (_phase, mut registrar) = registrar();
        declare(
            &mut registrar,
            "robot",
            f64::from(f32::MAX),
            f64::from(f32::MAX),
            i32::MAX,
            i32::MAX,
            &[],
        )
        .unwrap();
        declare(&mut registrar, "signed", 0.0, -1.0, i32::MIN, i32::MIN, &[]).unwrap();
        assert_eq!(registrar.general_entity_types.len(), 2);
        assert!(registrar.refused.is_empty());
    }

    #[test]
    fn client_only_package_refuses_entity_types() {
        crate::__private::initialize().unwrap();
        use crate::load_state::with_test_package_inventory;
        use crate::{PackageInfo, PackageKind};
        let packages = vec![PackageInfo::new(
            "amod".to_owned(),
            "1".to_owned(),
            PackageKind::Code,
            crate::Side::Client,
            vec!["shared".to_owned()],
            vec![],
        )];
        let _registration = RegistrationGuard::open();
        with_test_package_inventory(packages, || {
            let mut registrar = PartRegistrar::new("amod");
            assert_eq!(
                declare(&mut registrar, "shared:x", 0.8, 1.9, 8, 3, &[]),
                Err(Error::WrongSide)
            );
            assert!(registrar.general_entity_types.is_empty());
        });
    }

    #[test]
    fn served_by_entity_type_enforces_supplier_side() {
        crate::__private::initialize().unwrap();
        use crate::load_state::with_test_package_inventory;
        use crate::{PackageInfo, PackageKind};
        let packages = vec![
            PackageInfo::new(
                "host".to_owned(),
                "1".to_owned(),
                PackageKind::Code,
                crate::Side::Both,
                vec!["host".to_owned()],
                vec![],
            ),
            PackageInfo::new(
                "runes".to_owned(),
                "1".to_owned(),
                PackageKind::Content,
                crate::Side::Both,
                vec!["runes".to_owned()],
                vec![],
            )
            .with_served_by(Some("host".to_owned())),
            PackageInfo::new(
                "srv_runes".to_owned(),
                "1".to_owned(),
                PackageKind::Content,
                crate::Side::Server,
                vec!["srv_runes".to_owned()],
                vec![],
            )
            .with_served_by(Some("host".to_owned())),
        ];
        let _registration = crate::__private::RegistrationGuard::open();
        with_test_package_inventory(packages, || {
            let mut registrar = PartRegistrar::new("host");
            declare(&mut registrar, "runes:drone", 0.8, 1.9, 8, 3, &[]).unwrap();
            assert_eq!(
                declare(&mut registrar, "srv_runes:drone", 0.8, 1.9, 8, 3, &[]),
                Err(Error::WrongSide)
            );
            assert_eq!(registrar.general_entity_types.len(), 1);
            assert_eq!(registrar.general_entity_types[0].id, "runes:drone");
        });
    }

    /// The registry is frozen outside startup registration: a declaration
    /// then refuses with `Frozen`, whatever it names, and the frozen table is
    /// unchanged.
    #[test]
    pub(crate) fn general_entity_types_frozen_outside_registration() {
        crate::__private::initialize().unwrap();
        let mut registrar = PartRegistrar::new("mymod");
        assert_eq!(
            declare(&mut registrar, "robot", 0.8, 1.9, 8, 3, &[]),
            Err(Error::Frozen)
        );
        assert!(registrar.general_entity_types.is_empty());
        assert_eq!(registrar.refused.len(), 1);
    }

    /// A declaration from a mod the test inventory does not list stands: the
    /// side check refuses only a listed non-both-side package, exactly as the
    /// attribute registration's does.
    #[test]
    pub(crate) fn general_entity_types_unlisted_mod_declares() {
        crate::__private::initialize().unwrap();
        let (_phase, mut registrar) = registrar();
        declare(&mut registrar, "robot", 0.8, 1.9, 8, 3, &[]).unwrap();
        assert_eq!(registrar.general_entity_types.len(), 1);
        assert!(registrar.refused.is_empty());
    }

    /// A mod's `Hooks::register` receives `&mut dyn Registrar`, and the
    /// generated `Registrar` trait carries `GeneralTypesRegistrar` among its
    /// supertraits: the declaration reaches mods through the registrar they
    /// already hold, with no engine type leaking. Red if the supertrait
    /// wiring is lost: `declare_general_entity` would not resolve on
    /// `dyn Registrar`.
    /// Two mods may own one namespace (a manifest's `namespaces`), but their
    /// registrations run against the one registrar `register_all` drives
    /// (`parts/shared.rs`), so the second mod's declaration of the same key
    /// is a `Duplicate` at registration, with a refusal naming it — never a
    /// panic in the game thread at freeze. Red without the duplicate check:
    /// the second declaration would stand and the frozen registry would hit
    /// the unreachable.
    #[test]
    pub(crate) fn general_entity_types_two_mods_one_key_refuse_the_second() {
        crate::__private::initialize().unwrap();
        use crate::load_state::with_test_package_inventory;
        use crate::{PackageInfo, PackageKind};
        let packages = vec![
            PackageInfo::new(
                "amod".to_owned(),
                "1".to_owned(),
                PackageKind::Code,
                crate::Side::Both,
                vec!["shared".to_owned()],
                vec![],
            ),
            PackageInfo::new(
                "bmod".to_owned(),
                "1".to_owned(),
                PackageKind::Code,
                crate::Side::Both,
                vec!["shared".to_owned()],
                vec![],
            ),
        ];
        let _registration = RegistrationGuard::open();
        with_test_package_inventory(packages, || {
            let mut registrar = PartRegistrar::new("amod");
            declare(&mut registrar, "shared:x", 0.8, 1.9, 8, 3, &[]).unwrap();
            // What `register_all` does between mods: the same registrar, the
            // next mod's id.
            registrar.mod_id = "bmod";
            assert_eq!(
                declare(&mut registrar, "shared:x", 0.6, 1.2, 4, 2, &[]),
                Err(Error::Duplicate)
            );
            assert_eq!(registrar.general_entity_types.len(), 1);
            assert_eq!(registrar.general_entity_types[0].mod_id, "amod");
            assert_eq!(registrar.refused.len(), 1);
        });
    }

    #[test]
    pub(crate) fn declare_general_entity_reaches_mods_through_dyn_registrar() {
        crate::__private::initialize().unwrap();
        let _registration = RegistrationGuard::open();
        let mut registrar = PartRegistrar::new("mymod");
        let dynamic: &mut dyn crate::Registrar = &mut registrar;
        dynamic
            .declare_general_entity(
                "robot",
                GeneralEntityCategory::Misc,
                0.8,
                1.9,
                None,
                GeneralSavePolicy::Save,
                true,
                false,
                false,
                Some(PushReaction::IgnoreEntity),
                8,
                3,
                1,
                &[],
            )
            .unwrap();
        assert_eq!(registrar.general_entity_types.len(), 1);
        assert_eq!(
            registrar.general_entity_types[0].piston_push_reaction,
            PushReaction::IgnoreEntity
        );
    }
}
