//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl ArmPosesRegistrar for PartRegistrar {
    fn register_arm_pose(
        &mut self,
        name: &str,
        uses_both_arms: bool,
        affects_offhand: bool,
    ) -> crate::CustomArmPose {
        if crate::Identifier::parse(name).is_err()
            || !namespaced(name)
            || name.split_once(':').is_none_or(|(namespace, _)| {
                namespace == "minecraft" || !owns_namespace(self.mod_id, namespace)
            })
            || self.arm_poses.iter().any(|pose| pose.id == name)
        {
            self.refused.push(format!(
                "{} registered invalid or duplicate arm pose {name:?}",
                self.mod_id
            ));
            return mod_vocab::__private::parts_installation::arm_pose(u32::MAX, false, false);
        }
        let slot = self.arm_poses.len() as u32;
        self.arm_poses.push(PoseInfo {
            id: name.into(),
            both: uses_both_arms,
            offhand: affects_offhand,
        });
        mod_vocab::__private::parts_installation::arm_pose(slot, uses_both_arms, affects_offhand)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registrations_refuse_invalid_duplicate_and_foreign_names() {
        let mut registrar = PartRegistrar::new("skillet");
        let pose = registrar.register_arm_pose("skillet:flip", true, false);
        assert_eq!(pose.slot(), 0);
        for id in [
            "skillet:flip",
            "other:flip",
            "",
            "skillet:",
            "skillet:Bad Name",
        ] {
            assert_eq!(
                registrar.register_arm_pose(id, false, false).slot(),
                u32::MAX
            );
        }
        assert_eq!(registrar.refused.len(), 5);
        assert_eq!(registrar.arm_poses.len(), 1);
        assert!(registrar.arm_poses[0].both);
        assert!(!registrar.arm_poses[0].offhand);
    }

    #[test]
    fn custom_pose_wire_preserves_flags_without_the_registration_table() {
        use crate::wire::{Reader, Writer};
        let pose = crate::ArmPose::Custom(mod_vocab::__private::parts_installation::arm_pose(
            37, true, false,
        ));
        let mut output = Writer::new();
        output.put(&pose);
        let _empty_table = mod_vocab::__private::parts_installation::empty_pose_table();
        let decoded = Reader::decode::<crate::ArmPose>(output.bytes()).unwrap();
        assert_eq!(decoded, pose);
        assert!(decoded.two_handed());
        assert!(!decoded.affects_offhand());
        // Decoding preserves data; admission into the engine, not the codec,
        // checks membership in the load-owned registration table.
        assert!(
            mod_vocab::__private::parts_installation::arm_pose(u32::MAX, false, false)
                .flags()
                .is_none()
        );
    }

    #[test]
    fn installed_pose_flags_are_independent_and_invalid_slots_have_no_pose() {
        struct Mod;
        #[crate::__subscriptions]
        impl crate::Hooks for Mod {
            fn register(
                &self,
                registrar: &mut dyn crate::Registrar,
                _: &crate::RegistrationContext,
            ) {
                registrar.register_arm_pose("skillet:two", true, false);
                registrar.register_arm_pose("skillet:off", false, true);
            }
        }
        crate::testing::with_mods(
            vec![crate::ModEntry::new("skillet", Box::new(Mod))].into(),
            || {
                let two = crate::CustomArmPose::from_slot(0).unwrap();
                let off = crate::CustomArmPose::from_slot(1).unwrap();
                assert_eq!(two.flags(), Some((true, false)));
                assert!(crate::ArmPose::Custom(two).two_handed());
                assert!(!crate::ArmPose::Custom(two).affects_offhand());
                assert!(!crate::ArmPose::Custom(off).two_handed());
                assert!(crate::ArmPose::Custom(off).affects_offhand());
                assert!(crate::CustomArmPose::from_slot(u32::MAX).is_none());
            },
        );
    }

    #[test]
    fn served_by_arm_pose_delegates_to_serving_host_only() {
        let packages = vec![
            crate::PackageInfo::new(
                "host".into(),
                "1".into(),
                crate::PackageKind::Code,
                crate::Side::Both,
                vec!["host".into()],
                vec![],
            ),
            crate::PackageInfo::new(
                "other".into(),
                "1".into(),
                crate::PackageKind::Code,
                crate::Side::Both,
                vec!["other".into()],
                vec![],
            ),
            crate::PackageInfo::new(
                "runes".into(),
                "1".into(),
                crate::PackageKind::Content,
                crate::Side::Both,
                vec!["runes".into()],
                vec![],
            )
            .with_served_by(Some("host".into())),
        ];
        crate::load_state::with_test_package_inventory(packages, || {
            let mut registrar = PartRegistrar::new("host");
            let pose = registrar.register_arm_pose("runes:cast", true, false);
            assert_eq!(pose.slot(), 0);
            assert_eq!(
                registrar
                    .register_arm_pose("minecraft:cast", false, false)
                    .slot(),
                u32::MAX
            );
            registrar.mod_id = "other";
            assert_eq!(
                registrar
                    .register_arm_pose("runes:other_cast", false, false)
                    .slot(),
                u32::MAX
            );
            assert_eq!(registrar.arm_poses.len(), 1);
            assert_eq!(registrar.arm_poses[0].id, "runes:cast");
            assert_eq!(registrar.refused.len(), 2);
        });
    }
}
