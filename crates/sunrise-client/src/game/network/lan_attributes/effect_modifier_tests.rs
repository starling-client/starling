//! #478: a mob's effect attribute modifiers reach the attribute packet as the
//! stored modifiers they are, once, and a loaded effect (which stores none)
//! sends none.
use super::SpeedByType;
use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::mobs::{Mob, StoredEffect};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::packets::play::{AttributeModifier, AttributeSnapshot};

#[test]
#[ignore = "daily"]
fn the_attribute_packet_carries_a_given_effect_modifier_once() {
    with_game(|game| {
        let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mut given = Mob::new(12, pig.clone(), Vec3::ZERO, 0.0);
        given.add_effect("minecraft:speed", -1, 1);
        let mut loaded = Mob::new(13, pig, Vec3::ZERO, 0.0);
        loaded.load_effect(StoredEffect::new("minecraft:speed", -1, 1));
        let speed = game
            .motion
            .attributes
            .id("minecraft:movement_speed")
            .unwrap()
            .registry_id();
        let synced = |m: &Mob| -> Vec<AttributeSnapshot> {
            let mut out = Vec::new();
            game.mob_attributes(m, &mut SpeedByType::default(), &mut out);
            out.into_iter().filter(|s| s.attribute == speed).collect()
        };
        // One movement speed instance: the pig's 0.25 and the stored
        // `minecraft:effect.speed`, 0.2F x 2, add_multiplied_total (2).
        let given = synced(&given);
        assert_eq!(given.len(), 1, "{given:?}");
        assert_eq!(given[0].base, 0.25);
        assert_eq!(
            given[0].modifiers,
            [AttributeModifier {
                id: "minecraft:effect.speed".into(),
                amount: 0.400_000_005_960_464_5,
                operation: 2,
            }]
        );
        // A loaded effect has no modifier to send.
        let loaded = synced(&loaded);
        assert_eq!(loaded.len(), 1, "{loaded:?}");
        assert_eq!(loaded[0].base, 0.25);
        assert!(loaded[0].modifiers.is_empty(), "{loaded:?}");
    });
}

/// A spawned baby zombie given Speed II: the packet lists `minecraft:baby`
/// (`setBaby` at spawn) before `minecraft:effect.speed` (`onEffectAdded`
/// later), the instance's insertion order, each once.
#[test]
#[ignore = "daily"]
fn a_babys_modifier_comes_before_a_given_effects_in_the_packet() {
    with_game(|game| {
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut baby = Mob::new(12, zombie, Vec3::ZERO, 0.0);
        sunrise_client::mobs::finalize::set_baby(&mut baby, true);
        baby.add_effect("minecraft:speed", -1, 1);
        let speed = game
            .motion
            .attributes
            .id("minecraft:movement_speed")
            .unwrap()
            .registry_id();
        let mut out = Vec::new();
        game.mob_attributes(&baby, &mut SpeedByType::default(), &mut out);
        let rows: Vec<_> = out.iter().filter(|s| s.attribute == speed).collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(
            rows[0].modifiers,
            [
                AttributeModifier {
                    id: "minecraft:baby".into(),
                    amount: 0.5,
                    operation: 1,
                },
                AttributeModifier {
                    id: "minecraft:effect.speed".into(),
                    amount: 0.400_000_005_960_464_5,
                    operation: 2,
                },
            ]
        );
    });
}

/// A cat given Speed II that then dashes: `setSprinting` removes and adds
/// `minecraft:sprinting` each time the dash starts, so it comes after the
/// effect's modifier, each once.
#[test]
#[ignore = "daily"]
fn a_dash_after_a_given_effect_comes_after_it_in_the_packet() {
    with_game(|game| {
        let kind = game.res.mob_types().get("minecraft:cat").unwrap().clone();
        let mut cat = Mob::new(12, kind, Vec3::ZERO, 0.0);
        cat.add_effect("minecraft:speed", -1, 1);
        cat.extra.sprinting = true;
        let speed = game
            .motion
            .attributes
            .id("minecraft:movement_speed")
            .unwrap()
            .registry_id();
        let mut out = Vec::new();
        game.mob_attributes(&cat, &mut SpeedByType::default(), &mut out);
        let rows: Vec<_> = out.iter().filter(|s| s.attribute == speed).collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(
            rows[0].modifiers,
            [
                AttributeModifier {
                    id: "minecraft:effect.speed".into(),
                    amount: 0.400_000_005_960_464_5,
                    operation: 2,
                },
                AttributeModifier {
                    id: "minecraft:sprinting".into(),
                    amount: 0.300_000_011_920_928_96,
                    operation: 2,
                },
            ]
        );
    });
}

/// #1988: a zombie given Jump Boost II and Absorption II sends each stored
/// modifier once, on its attribute's instance (base 3 and 0): 2.0 and 8.0,
/// `add_value` (0). A zombie that loaded the same effects stored none, and
/// no instance of either is sent for it. `waypoint_transmit_range` is not
/// syncable, so Invisibility's modifier is never sent.
#[test]
#[ignore = "daily"]
fn the_attribute_packet_carries_given_jump_boost_and_absorption_modifiers_once() {
    with_game(|game| {
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut given = Mob::new(12, zombie.clone(), Vec3::ZERO, 0.0);
        let mut loaded = Mob::new(13, zombie, Vec3::ZERO, 0.0);
        for (effect, amplifier) in [
            ("minecraft:jump_boost", 1),
            ("minecraft:absorption", 1),
            ("minecraft:invisibility", 0),
        ] {
            given.add_effect(effect, -1, amplifier);
            loaded.load_effect(StoredEffect::new(effect, -1, amplifier));
        }
        let id = |name: &str| game.motion.attributes.id(name).unwrap().registry_id();
        let (safe_fall, max_absorption, range) = (
            id("minecraft:safe_fall_distance"),
            id("minecraft:max_absorption"),
            id("minecraft:waypoint_transmit_range"),
        );
        let synced = |m: &Mob| -> Vec<AttributeSnapshot> {
            let mut out = Vec::new();
            game.mob_attributes(m, &mut SpeedByType::default(), &mut out);
            out
        };
        let rows = synced(&given);
        let row = |attribute: i32| {
            let found: Vec<_> = rows.iter().filter(|s| s.attribute == attribute).collect();
            assert_eq!(found.len(), 1, "{rows:?}");
            found[0].clone()
        };
        let jump = row(safe_fall);
        assert_eq!(jump.base, 3.0);
        assert_eq!(
            jump.modifiers,
            [AttributeModifier {
                id: "minecraft:effect.jump_boost".into(),
                amount: 2.0,
                operation: 0,
            }]
        );
        let absorption = row(max_absorption);
        assert_eq!(absorption.base, 0.0);
        assert_eq!(
            absorption.modifiers,
            [AttributeModifier {
                id: "minecraft:effect.absorption".into(),
                amount: 8.0,
                operation: 0,
            }]
        );
        assert!(rows.iter().all(|s| s.attribute != range), "{rows:?}");
        let rows = synced(&loaded);
        assert!(
            rows.iter()
                .all(|s| ![safe_fall, max_absorption, range].contains(&s.attribute)),
            "{rows:?}"
        );
    });
}
