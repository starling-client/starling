#![cfg(test)]
#[test]
fn client_only_attribute_does_not_shift_both_side_registry() {
    use crate::Sentiment;
    use crate::parts::{AttributesRegistrar, PartRegistrar};
    let packages = [("alpha", crate::Side::Client), ("zeta", crate::Side::Both)]
        .into_iter()
        .map(|(id, side)| {
            PackageInfo::new(
                id.to_owned(),
                "1".to_owned(),
                PackageKind::Code,
                side,
                vec![id.to_owned()],
                vec![],
            )
        })
        .collect::<Vec<_>>();
    crate::load_state::with_test_package_inventory(packages, || {
        let mut alpha = PartRegistrar::new("alpha");
        let mut zeta = PartRegistrar::new("zeta");
        assert_eq!(
            alpha.declare_attribute(
                "decoration",
                0.0,
                0.0,
                1.0,
                false,
                Sentiment::Neutral,
                Default::default()
            ),
            Err(crate::AttributeRegistrationError::WrongSide)
        );
        assert!(
            zeta.declare_attribute(
                "mana",
                0.0,
                0.0,
                1.0,
                true,
                Sentiment::Positive,
                Default::default()
            )
            .is_ok()
        );
        assert_eq!(
            alpha.widen_attribute_bounds("minecraft:max_health", 1.0, 1e6),
            Err(crate::AttributeRegistrationError::WrongSide)
        );
        assert!(
            zeta.widen_attribute_bounds("minecraft:max_health", 1.0, 1e6)
                .is_ok()
        );
        assert!(alpha.attributes.is_empty() && alpha.attribute_widenings.is_empty());
        assert_eq!(zeta.attributes[0].id, "zeta:mana");
    });
}

#[test]
fn served_by_attribute_enforces_supplier_side() {
    use crate::Sentiment;
    use crate::parts::{AttributesRegistrar, PartRegistrar};
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
    crate::load_state::with_test_package_inventory(packages, || {
        let mut registrar = PartRegistrar::new("host");
        registrar
            .declare_attribute(
                "runes:mana",
                10.0,
                0.0,
                100.0,
                true,
                Sentiment::Positive,
                Default::default(),
            )
            .unwrap();
        assert_eq!(
            registrar.declare_attribute(
                "srv_runes:mana",
                10.0,
                0.0,
                100.0,
                true,
                Sentiment::Positive,
                Default::default(),
            ),
            Err(crate::AttributeRegistrationError::WrongSide)
        );
        assert_eq!(
            registrar.widen_attribute_bounds("srv_runes:mana", 0.0, 200.0),
            Err(crate::AttributeRegistrationError::WrongSide)
        );
        registrar
            .widen_attribute_bounds("runes:mana", 0.0, 200.0)
            .unwrap();
        assert_eq!(registrar.attributes.len(), 1);
        assert_eq!(registrar.attributes[0].id, "runes:mana");
        assert_eq!(registrar.attribute_widenings.len(), 1);
        assert_eq!(registrar.attribute_widenings[0].id, "runes:mana");
    });
}

#[cfg(test)]
use super::*;
