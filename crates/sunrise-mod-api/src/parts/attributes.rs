//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl AttributesRegistrar for PartRegistrar {
    fn declare_attribute(
        &mut self,
        path: &str,
        default: f64,
        min: f64,
        max: f64,
        syncable: bool,
        sentiment: Sentiment,
        tooltip: crate::AttributeTooltip,
    ) -> Result<(), crate::AttributeRegistrationError> {
        let id = match self.declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return Err(crate::AttributeRegistrationError::InvalidId);
            }
        };
        let result = (|| {
            if self.declaration_side_not_both(&id) {
                return Err(crate::AttributeRegistrationError::WrongSide);
            }
            if self.attributes.iter().any(|declared| declared.id == id)
                || self.has_builtin_entry("minecraft:attribute", &id)
            {
                return Err(crate::AttributeRegistrationError::Duplicate);
            }
            if ![default, min, max].into_iter().all(f64::is_finite) {
                return Err(crate::AttributeRegistrationError::NotFinite);
            }
            if min > max || default < min || default > max {
                return Err(crate::AttributeRegistrationError::InvalidBounds);
            }
            if tooltip
                .base_modifier
                .as_deref()
                .is_some_and(|id| !id.contains(':') || crate::Identifier::parse(id).is_err())
            {
                return Err(crate::AttributeRegistrationError::InvalidId);
            }
            self.attributes.push(AttributeDeclaration {
                mod_id: self.mod_id.to_owned(),
                id,
                default,
                min,
                max,
                syncable,
                sentiment,
                tooltip,
            });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused
                .push(format!("mod {} attribute {path:?}: {error:?}", self.mod_id));
        }
        result
    }

    fn widen_attribute_bounds(
        &mut self,
        id: &str,
        min: f64,
        max: f64,
    ) -> Result<(), crate::AttributeRegistrationError> {
        let result = if self.declaration_side_not_both(id) {
            Err(crate::AttributeRegistrationError::WrongSide)
        } else if !namespaced(id) {
            Err(crate::AttributeRegistrationError::InvalidId)
        } else if !min.is_finite() || !max.is_finite() {
            Err(crate::AttributeRegistrationError::NotFinite)
        } else if min > max {
            Err(crate::AttributeRegistrationError::InvalidBounds)
        } else {
            self.attribute_widenings.push(AttributeBoundsWidening {
                mod_id: self.mod_id.to_owned(),
                id: id.to_owned(),
                min,
                max,
            });
            Ok(())
        };
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} attribute bounds of {id:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AttributeRegistrationError;

    /// `RangedAttribute`'s own checks and the registration's: explicit
    /// namespaces keep their key with separate mod attribution, while
    /// malformed ids, duplicates (including built-in attributes), non-finite
    /// values and bounds the default is not inside all refuse, naming the mod;
    /// the good declarations stand.
    #[test]
    pub(crate) fn attribute_declarations_validate() {
        let data = mod_vocab::__private::RegistrationData::new(
            [("minecraft:attribute".into(), vec!["minecraft:armor".into()])].into(),
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
            let mut registrar = PartRegistrar::new("mymod");
            registrar
                .declare_attribute(
                    "mana",
                    10.0,
                    0.0,
                    100.0,
                    true,
                    Sentiment::Neutral,
                    Default::default(),
                )
                .unwrap();
            // Explicit namespaces succeed with separate mod attribution, while
            // built-in attribute keys refuse as duplicates.
            assert_eq!(
                registrar.declare_attribute(
                    "minecraft:armor",
                    0.0,
                    0.0,
                    30.0,
                    true,
                    positive(),
                    Default::default()
                ),
                Err(AttributeRegistrationError::Duplicate)
            );
            registrar
                .declare_attribute(
                    "foreign:mana",
                    0.0,
                    0.0,
                    1.0,
                    false,
                    positive(),
                    Default::default(),
                )
                .unwrap();
            registrar
                .declare_attribute(
                    "minecraft:custom_mana",
                    0.0,
                    0.0,
                    1.0,
                    false,
                    positive(),
                    Default::default(),
                )
                .unwrap();
            assert_eq!(
                registrar.declare_attribute(
                    "Not An Id",
                    0.0,
                    0.0,
                    1.0,
                    false,
                    positive(),
                    Default::default()
                ),
                Err(AttributeRegistrationError::InvalidId)
            );
            // The same attribute twice, whatever declared it, is a duplicate.
            assert_eq!(
                registrar.declare_attribute(
                    "mana",
                    1.0,
                    0.0,
                    1.0,
                    false,
                    positive(),
                    Default::default()
                ),
                Err(AttributeRegistrationError::Duplicate)
            );
            // RangedAttribute's own bounds checks.
            assert_eq!(
                registrar.declare_attribute(
                    "rage",
                    5.0,
                    10.0,
                    0.0,
                    false,
                    positive(),
                    Default::default()
                ),
                Err(AttributeRegistrationError::InvalidBounds)
            );
            assert_eq!(
                registrar.declare_attribute(
                    "rage",
                    0.0,
                    1.0,
                    10.0,
                    false,
                    positive(),
                    Default::default()
                ),
                Err(AttributeRegistrationError::InvalidBounds)
            );
            assert_eq!(
                registrar.declare_attribute(
                    "rage",
                    20.0,
                    1.0,
                    10.0,
                    false,
                    positive(),
                    Default::default()
                ),
                Err(AttributeRegistrationError::InvalidBounds)
            );
            // A non-finite default or bound.
            assert_eq!(
                registrar.declare_attribute(
                    "rage",
                    f64::NAN,
                    0.0,
                    1.0,
                    false,
                    positive(),
                    Default::default()
                ),
                Err(AttributeRegistrationError::NotFinite)
            );
            assert_eq!(
                registrar.declare_attribute(
                    "rage",
                    0.5,
                    f64::NEG_INFINITY,
                    1.0,
                    false,
                    positive(),
                    Default::default()
                ),
                Err(AttributeRegistrationError::NotFinite)
            );
            assert_eq!(
                registrar.declare_attribute(
                    "rage",
                    0.5,
                    0.0,
                    f64::INFINITY,
                    false,
                    positive(),
                    Default::default()
                ),
                Err(AttributeRegistrationError::NotFinite)
            );
            // A base modifier must be a namespaced id; a good one is kept with the display.
            let tooltip = |base: &str| crate::AttributeTooltip {
                display: crate::AttributeDisplay::Percentage,
                base_modifier: Some(base.to_owned()),
            };
            assert_eq!(
                registrar.declare_attribute(
                    "swim",
                    0.0,
                    0.0,
                    1.0,
                    true,
                    positive(),
                    tooltip("base")
                ),
                Err(AttributeRegistrationError::InvalidId)
            );
            registrar
                .declare_attribute(
                    "swim",
                    0.0,
                    0.0,
                    1.0,
                    true,
                    positive(),
                    tooltip("mymod:base_swim"),
                )
                .unwrap();
            assert_eq!(registrar.attributes[3].tooltip, tooltip("mymod:base_swim"));
            registrar.attributes.truncate(3);
            // The refusals name the mod; the good declarations stand with owner attribution.
            assert_eq!(registrar.attributes.len(), 3);
            let declared = &registrar.attributes[0];
            assert_eq!(
                (declared.mod_id.as_str(), declared.id.as_str()),
                ("mymod", "mymod:mana")
            );
            assert_eq!(
                (declared.default, declared.min, declared.max),
                (10.0, 0.0, 100.0)
            );
            assert!(declared.syncable);
            assert_eq!(declared.sentiment, Sentiment::Neutral);
            assert_eq!(declared.tooltip, crate::AttributeTooltip::default());
            assert_eq!(
                (
                    registrar.attributes[1].mod_id.as_str(),
                    registrar.attributes[1].id.as_str()
                ),
                ("mymod", "foreign:mana")
            );
            assert_eq!(
                (
                    registrar.attributes[2].mod_id.as_str(),
                    registrar.attributes[2].id.as_str()
                ),
                ("mymod", "minecraft:custom_mana")
            );
            assert!(
                registrar
                    .refused
                    .iter()
                    .all(|refusal| refusal.contains("mod mymod"))
            );
        });
    }

    /// Widening asks are checked as they come: a non-finite bound, `min >
    /// max` and a malformed id refuse, naming the mod; a good ask stands,
    /// whatever namespace it names (vanilla's, another mod's).
    #[test]
    fn attribute_bounds_widenings_validate() {
        let mut registrar = PartRegistrar::new("rpg");
        registrar
            .widen_attribute_bounds("minecraft:max_health", 1.0, 1e6)
            .unwrap();
        for (min, max) in [
            (f64::NAN, 1e6),
            (1.0, f64::NAN),
            (f64::NEG_INFINITY, 1e6),
            (1.0, f64::INFINITY),
        ] {
            assert_eq!(
                registrar.widen_attribute_bounds("minecraft:attack_damage", min, max),
                Err(AttributeRegistrationError::NotFinite)
            );
        }
        assert_eq!(
            registrar.widen_attribute_bounds("minecraft:attack_damage", 10.0, 0.0),
            Err(AttributeRegistrationError::InvalidBounds)
        );
        assert_eq!(
            registrar.widen_attribute_bounds("Not An Id", 0.0, 1.0),
            Err(AttributeRegistrationError::InvalidId)
        );
        assert_eq!(
            registrar.attribute_widenings,
            [AttributeBoundsWidening {
                mod_id: "rpg".into(),
                id: "minecraft:max_health".into(),
                min: 1.0,
                max: 1e6,
            }]
        );
        assert_eq!(registrar.refused.len(), 6);
        assert!(registrar.refused.iter().all(|r| r.contains("mod rpg")));
    }

    #[test]
    fn attribute_sentiment_wire_round_trip() {
        use crate::wire::{Reader, Wire, Writer};
        for sentiment in [Sentiment::Positive, Sentiment::Negative, Sentiment::Neutral] {
            let mut writer = Writer::new();
            sentiment.put(&mut writer);
            let bytes = writer.into_bytes();
            assert_eq!(
                Sentiment::take(&mut Reader::new(&bytes)).unwrap(),
                sentiment
            );
        }
    }

    fn positive() -> Sentiment {
        Sentiment::default()
    }
}
