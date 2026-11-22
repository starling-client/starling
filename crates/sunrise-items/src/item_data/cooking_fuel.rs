//! `CookingFuel.CODEC` and its decoded runtime value.
use crate::{
    items::{ItemStack, Items},
    properties::resolvable::{self, Float, Int},
};
use std::sync::Arc;
use sunrise_core::nbt::Tag;

pub(super) const ID: &str = "minecraft:cooking_fuel";
pub(super) const REMOVED: &str = "!minecraft:cooking_fuel";

#[derive(Debug)]
pub enum Resolvable<T> {
    Constant(T),
    Reference(String),
}

#[derive(Debug)]
pub struct CookingFuel {
    pub burn_time: Resolvable<i32>,
    pub speed_multiplier: Resolvable<f32>,
}

#[derive(Clone, Debug)]
pub(super) enum Patch {
    Set(Arc<CookingFuel>),
    Unavailable,
}

impl Patch {
    pub(super) fn from_value(value: &Tag) -> Self {
        match fields(value) {
            Ok((burn, speed)) => Self::Set(Arc::new(CookingFuel {
                burn_time: match burn {
                    Int::Constant(value) => Resolvable::Constant(value),
                    Int::Reference(key) => Resolvable::Reference(key.to_string()),
                },
                speed_multiplier: match speed {
                    Float::Constant(value) => Resolvable::Constant(value),
                    Float::Reference(key) => Resolvable::Reference(key.to_string()),
                },
            })),
            Err(_) => Self::Unavailable,
        }
    }
}

fn fields(value: &Tag) -> Result<(Int<'_>, Float<'_>), &'static str> {
    if !matches!(value, Tag::Compound(_)) {
        return Err("Expected cooking fuel map");
    }
    let burn = value.get("burn_time").ok_or("Missing burn_time")?;
    let speed = value
        .get("speed_multiplier")
        .ok_or("Missing speed_multiplier")?;
    Ok((
        resolvable::int(burn).ok_or("Invalid burn_time: expected number or resource key")?,
        resolvable::float(speed)
            .ok_or("Invalid speed_multiplier: expected number or resource key")?,
    ))
}

/// `ItemStack.get(DataComponents.COOKING_FUEL)`: read the retained payload, or its
/// prototype if the stack neither replaces nor removes the component.
pub fn for_stack<'a>(items: &'a Items, stack: &'a ItemStack) -> Option<&'a CookingFuel> {
    match stack
        .data()
        .and_then(|data| data.cooking_fuel.as_ref())
        .or_else(|| {
            items
                .get(&stack.item)?
                .component_prototype
                .data()?
                .cooking_fuel
                .as_ref()
        })? {
        Patch::Set(fuel) => Some(fuel),
        Patch::Unavailable => None,
    }
}

/// Decode the two required fields, ignoring unknown fields; neither has a default or range.
/// Resource keys are identifiers, not registry lookups, and constants may be non-finite.
pub fn decode(value: &Tag) -> Result<Tag, String> {
    let (burn, speed) = fields(value)?;
    Ok(Tag::compound([
        ("burn_time", burn.to_tag()),
        ("speed_multiplier", speed.to_tag()),
    ]))
}

/// Compare live retained payloads with no allocation, serialization or UTF-16 conversion.
pub fn equal(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    match (
        a.and_then(|v| fields(v).ok()),
        b.and_then(|v| fields(v).ok()),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::JavaString;

    fn fuel(burn: Tag, speed: Tag) -> Tag {
        Tag::compound([("burn_time", burn), ("speed_multiplier", speed)])
    }

    #[test]
    fn cooking_fuel_requires_record_fields_but_ignores_unknowns() {
        for value in [
            Tag::Int(10),
            Tag::compound([("burn_time", Tag::Int(10))]),
            Tag::compound([("speed_multiplier", Tag::Int(1))]),
            fuel(Tag::List(vec![]), Tag::Int(1)),
            fuel(Tag::Int(1), Tag::String("BAD".into())),
            fuel(Tag::String("..:bad".into()), Tag::Int(1)),
            fuel(Tag::String(JavaString(vec![0xd800])), Tag::Int(1)),
        ] {
            assert!(decode(&value).is_err(), "{value:?}");
        }
        let value = Tag::compound([
            ("burn_time", Tag::Short(-2)),
            ("speed_multiplier", Tag::Double(0.5_f64.to_bits())),
            ("ignored", Tag::Int(99)),
        ]);
        assert_eq!(
            decode(&value).unwrap(),
            fuel(Tag::Int(-2), Tag::Float(0x3f00_0000))
        );
        assert_eq!(
            decode(&fuel(
                Tag::String(":fuel/test".into()),
                Tag::String("speed".into())
            ))
            .unwrap(),
            fuel(
                Tag::String("minecraft:fuel/test".into()),
                Tag::String("minecraft:speed".into())
            )
        );
    }

    #[test]
    fn cooking_fuel_preserves_boxed_numeric_coercion_and_float_record_equality() {
        for (input, expected) in [
            (Tag::Byte(-1), -1),
            (Tag::Short(-2), -2),
            (Tag::Int(i32::MIN), i32::MIN),
            (Tag::Long(4_294_967_297), 1),
            (Tag::Float(3.9_f32.to_bits()), 3),
            (Tag::Double((-3.9_f64).to_bits()), -3),
            (Tag::Double(f64::NAN.to_bits()), 0),
            (Tag::Double(f64::INFINITY.to_bits()), i32::MAX),
            (Tag::Double(f64::NEG_INFINITY.to_bits()), i32::MIN),
        ] {
            assert_eq!(
                decode(&fuel(input, Tag::Byte(1))).unwrap(),
                fuel(Tag::Int(expected), Tag::Float(0x3f80_0000))
            );
        }
        for (input, bits) in [
            (Tag::Byte(-1), 0xbf80_0000),
            (Tag::Short(2), 0x4000_0000),
            (Tag::Int(16_777_217), 0x4b80_0000),
            (Tag::Long(16_777_217), 0x4b80_0000),
            (Tag::Float(0x8000_0000), 0x8000_0000),
            (Tag::Double(f64::MAX.to_bits()), 0x7f80_0000),
            (Tag::Double(f64::NEG_INFINITY.to_bits()), 0xff80_0000),
            (Tag::Float(0xff80_0001), 0xff80_0001),
        ] {
            assert_eq!(
                decode(&fuel(Tag::Int(0), input)).unwrap(),
                fuel(Tag::Int(0), Tag::Float(bits))
            );
        }
        let nan_a = fuel(Tag::Int(1), Tag::Float(0x7f80_0001));
        let nan_b = fuel(Tag::Long(1), Tag::Double(f64::NAN.to_bits()));
        assert!(equal(Some(&nan_a), Some(&nan_b)));
        let positive = fuel(Tag::Int(1), Tag::Float(0));
        let negative = fuel(Tag::Int(1), Tag::Float(0x8000_0000));
        assert!(!equal(Some(&positive), Some(&negative)));
        let reference = fuel(Tag::String("1".into()), Tag::Int(1));
        assert!(!equal(
            Some(&reference),
            Some(&fuel(Tag::Int(1), Tag::Int(1)))
        ));
    }
}
