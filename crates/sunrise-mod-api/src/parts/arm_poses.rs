//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl ArmPosesRegistrar for PartRegistrar {
    fn register_arm_pose(
        &mut self,
        name: &str,
        uses_both_arms: bool,
        affects_offhand: bool,
    ) -> crate::CustomArmPose {
        if crate::Identifier::parse_namespaced(name).is_none() {
            self.refused.push(format!(
                "{} registered invalid arm pose {name:?}",
                self.mod_id
            ));
            return mod_vocab::__private::parts_installation::arm_pose(u32::MAX, false, false);
        }
        if let Some(previous) = self.arm_poses.iter().find(|pose| pose.id == name) {
            self.refused.push(format!(
                "{} registered duplicate arm pose {name:?} (already registered by {})",
                self.mod_id, previous.owner
            ));
            return mod_vocab::__private::parts_installation::arm_pose(u32::MAX, false, false);
        }
        let slot = self.arm_poses.len() as u32;
        self.arm_poses.push(PoseInfo {
            id: name.into(),
            owner: self.mod_id,
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
    fn registrations_accept_explicit_namespaces_and_refuse_invalid_or_duplicate_names() {
        let mut registrar = PartRegistrar::new("skillet");
        let own = registrar.register_arm_pose("skillet:flip", true, false);
        let foreign = registrar.register_arm_pose("other:flip", false, true);
        let vanilla_ns = registrar.register_arm_pose("minecraft:flip", false, false);
        assert_eq!((own.slot(), foreign.slot(), vanilla_ns.slot()), (0, 1, 2));
        for id in ["skillet:flip", "", "bare", "skillet:", "skillet:Bad Name"] {
            assert_eq!(
                registrar.register_arm_pose(id, false, false).slot(),
                u32::MAX
            );
        }
        registrar.mod_id = "second";
        assert_eq!(
            registrar
                .register_arm_pose("other:flip", false, false)
                .slot(),
            u32::MAX
        );
        assert_eq!(registrar.refused.len(), 6);
        assert!(registrar.refused[0].contains("already registered by skillet"));
        assert!(registrar.refused[5].contains("second"));
        assert!(registrar.refused[5].contains("already registered by skillet"));
        assert_eq!(registrar.arm_poses.len(), 3);
        assert_eq!(registrar.arm_poses[0].owner, "skillet");
        assert!(registrar.arm_poses[0].both);
        assert!(!registrar.arm_poses[0].offhand);
        assert_eq!(registrar.arm_poses[1].id, "other:flip");
        assert_eq!(registrar.arm_poses[1].owner, "skillet");
        assert_eq!(registrar.arm_poses[2].id, "minecraft:flip");
        assert_eq!(registrar.arm_poses[2].owner, "skillet");
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
    fn arm_pose_owner_is_the_registrant_not_the_namespace_supplier() {
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
                1
            );
            registrar.mod_id = "other";
            assert_eq!(
                registrar
                    .register_arm_pose("runes:other_cast", false, false)
                    .slot(),
                2
            );
            assert_eq!(
                registrar
                    .register_arm_pose("runes:cast", false, true)
                    .slot(),
                u32::MAX
            );
            assert_eq!(
                registrar
                    .arm_poses
                    .iter()
                    .map(|p| (p.id.as_str(), p.owner))
                    .collect::<Vec<_>>(),
                [
                    ("runes:cast", "host"),
                    ("minecraft:cast", "host"),
                    ("runes:other_cast", "other")
                ]
            );
            assert_eq!(
                registrar.refused,
                [
                    r#"other registered duplicate arm pose "runes:cast" (already registered by host)"#
                ]
            );
        });
    }
}
