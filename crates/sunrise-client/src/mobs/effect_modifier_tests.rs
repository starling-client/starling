//! #478: a mob's effect attribute modifiers (`minecraft:effect.<name>`) are
//! real modifiers in its attribute store, put on when an effect is added or
//! its active instance changes (`LivingEntity.onEffectAdded` /
//! `onEffectUpdated` -> `MobEffect.addAttributeModifiers`) and taken off when
//! it ends (`onEffectsRemoved` -> `removeAttributeModifiers`). Loading
//! `active_effects` adds none. The literal values are vanilla's: Speed is
//! `0.2F` widened to a double (0.20000000298023224) per level,
//! `add_multiplied_total`, on a pig's 0.25 movement speed.
use super::tick_effects;
use crate::entity_nbt::{mob_from_nbt, mob_to_nbt};
use crate::mobs::finalize::{self, Attr, Op};
use crate::mobs::{Mob, MobType, Species, sensor_scan_tests::kind};
use std::collections::HashMap;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;

const SPEED: &str = "minecraft:effect.speed";

fn mob_type(id: &str, species: Species, defaults: &[(&str, f64)]) -> Arc<MobType> {
    let mut t = (*kind()).clone();
    t.id = id.into();
    t.species = species;
    for (name, v) in defaults {
        match *name {
            "minecraft:max_health" => t.max_health = *v as f32,
            "minecraft:movement_speed" => t.speed = *v,
            "minecraft:attack_damage" => t.attack_damage = *v as f32,
            _ => {}
        }
    }
    t.attributes = Arc::new(
        defaults
            .iter()
            .map(|(name, v)| ((*name).to_owned(), *v))
            .collect(),
    );
    Arc::new(t)
}
/// A pig's registered defaults: no attack damage instance.
fn pig_type() -> Arc<MobType> {
    mob_type(
        "minecraft:pig",
        Species::Pig,
        &[
            ("minecraft:max_health", 10.0),
            ("minecraft:movement_speed", 0.25),
        ],
    )
}
/// A zombie's registered defaults.
fn zombie_type() -> Arc<MobType> {
    mob_type(
        "minecraft:zombie",
        Species::Zombie,
        &[
            ("minecraft:max_health", 20.0),
            ("minecraft:movement_speed", 0.23),
            ("minecraft:attack_damage", 3.0),
            ("minecraft:safe_fall_distance", 3.0),
            ("minecraft:max_absorption", 0.0),
            ("minecraft:waypoint_transmit_range", 0.0),
        ],
    )
}
fn pig() -> Mob {
    Mob::new(1, pig_type(), Vec3::ZERO, 0.0)
}
fn zombie() -> Mob {
    let mut m = Mob::new(2, zombie_type(), Vec3::ZERO, 0.0);
    m.health = 20.0;
    m
}
/// The stored modifiers with `id` on `a`, as (amount, operation).
fn stored(m: &Mob, a: Attr, id: &str) -> Vec<(f64, Op)> {
    m.extra.attributes.as_ref().map_or(Vec::new(), |s| {
        s.modifiers
            .iter()
            .filter(|x| x.attr == a && x.id == id)
            .map(|x| (x.amount, x.op))
            .collect()
    })
}
fn tick(m: &mut Mob, times: usize) {
    for _ in 0..times {
        tick_effects(
            m,
            crate::test_resources::server_loop_context(),
            &mut crate::health::TestLevel::new(0).level(),
            None,
        );
    }
}
fn types() -> HashMap<String, Arc<MobType>> {
    HashMap::from([
        ("minecraft:pig".to_owned(), pig_type()),
        ("minecraft:zombie".to_owned(), zombie_type()),
    ])
}

/// The issue's first row: a NoAI pig loaded with an infinite Speed II reads
/// 0.25 (`get 1000` = 250) and has no `minecraft:effect.speed` modifier.
#[test]
fn a_loaded_effect_adds_no_modifier() {
    let tag = sunrise_core::nbt::parse_snbt(
        r#"{id:"minecraft:pig",Pos:[0.5d,64d,0.5d],NoAI:1b,active_effects:[{id:"minecraft:speed",amplifier:1b,duration:-1}]}"#,
    )
    .unwrap()
    .0;
    let m = mob_from_nbt(&tag, &types(), 1, None).unwrap();
    assert!(m.has_effect("minecraft:speed"));
    assert!(stored(&m, Attr::MovementSpeed, SPEED).is_empty());
    assert_eq!(finalize::value(&m, Attr::MovementSpeed), 0.25);
    assert_eq!(m.attrs.speed, 0.25);
    assert_eq!(m.movement_speed(), 0.25);
    assert_eq!(
        (finalize::value(&m, Attr::MovementSpeed) * 1000.0) as i32,
        250
    );
}

/// The issue's given rows: `/effect give ... speed infinite 1` stores 0.4
/// (`modifier value get ... 1000` = 400), `add_multiplied_total`, and the
/// value is 0.35 (`get 1000` = 350); the base stays 0.25.
#[test]
fn a_given_effect_stores_its_modifier() {
    let mut m = pig();
    m.add_effect("minecraft:speed", -1, 1);
    assert_eq!(
        stored(&m, Attr::MovementSpeed, SPEED),
        [(0.400_000_005_960_464_5, Op::AddMultipliedTotal)]
    );
    assert_eq!(
        finalize::modifier_amount(&m, Attr::MovementSpeed, SPEED).map(|v| (v * 1000.0) as i32),
        Some(400)
    );
    assert_eq!(
        finalize::value(&m, Attr::MovementSpeed),
        0.350_000_001_490_116_1
    );
    assert_eq!(
        (finalize::value(&m, Attr::MovementSpeed) * 1000.0) as i32,
        350
    );
    assert_eq!(finalize::base_value(&m, Attr::MovementSpeed), 0.25);
    // Movement reads the attribute once: the modifier is not applied again.
    assert_eq!(m.attrs.speed, 0.350_000_001_490_116_1);
    assert_eq!(m.movement_speed(), 0.350_000_001_490_116_1);
}

/// `tickEffects` -> `onEffectsRemoved`: the effect running out takes its
/// modifier with it.
#[test]
fn the_modifier_leaves_when_the_effect_runs_out() {
    let mut m = pig();
    m.add_effect("minecraft:speed", 3, 1);
    assert_eq!(stored(&m, Attr::MovementSpeed, SPEED).len(), 1);
    tick(&mut m, 4);
    assert!(m.extra.effects.is_empty());
    assert!(stored(&m, Attr::MovementSpeed, SPEED).is_empty());
    assert_eq!(finalize::value(&m, Attr::MovementSpeed), 0.25);
    assert_eq!(m.attrs.speed, 0.25);
}

/// `addEffect` -> `onEffectUpdated(true)`: removeAttributeModifiers, then
/// addAttributeModifiers with the new amplifier; one modifier, never two.
#[test]
fn an_amplifier_change_replaces_the_amount() {
    let mut m = pig();
    m.add_effect("minecraft:speed", 100, 0);
    assert_eq!(
        stored(&m, Attr::MovementSpeed, SPEED),
        [(0.200_000_002_980_232_24, Op::AddMultipliedTotal)]
    );
    m.add_effect("minecraft:speed", 100, 2);
    assert_eq!(
        stored(&m, Attr::MovementSpeed, SPEED),
        [(0.600_000_008_940_696_7, Op::AddMultipliedTotal)]
    );
    assert_eq!(
        finalize::value(&m, Attr::MovementSpeed),
        0.400_000_002_235_174_2
    );
}

/// `tickServer`'s hidden instance taking over runs `onEffectUpdated(true)`:
/// the amount follows the amplifier that is active now.
#[test]
fn a_hidden_instance_taking_over_brings_its_amount() {
    let mut m = pig();
    m.add_effect("minecraft:speed", 200, 0);
    m.add_effect("minecraft:speed", 2, 1);
    assert_eq!(
        stored(&m, Attr::MovementSpeed, SPEED),
        [(0.400_000_005_960_464_5, Op::AddMultipliedTotal)]
    );
    tick(&mut m, 2);
    assert_eq!(m.extra.effects[0].amplifier, 0);
    assert_eq!(
        stored(&m, Attr::MovementSpeed, SPEED),
        [(0.200_000_002_980_232_24, Op::AddMultipliedTotal)]
    );
    assert_eq!(
        finalize::value(&m, Attr::MovementSpeed),
        0.300_000_000_745_058_06
    );
}

/// `removeEffect` (`/effect clear <mob> <effect>`, the illusioner's and the
/// trader's own removals) takes off that effect's modifier only.
#[test]
fn removing_one_effect_takes_off_its_modifier_only() {
    let mut m = zombie();
    m.add_effect("minecraft:speed", -1, 0);
    m.add_effect("minecraft:strength", -1, 0);
    assert_eq!(finalize::value(&m, Attr::AttackDamage), 6.0);
    assert!(m.retain_effects(|e| e.id != "minecraft:speed"));
    assert!(stored(&m, Attr::MovementSpeed, SPEED).is_empty());
    assert_eq!(finalize::value(&m, Attr::MovementSpeed), 0.23);
    assert_eq!(
        stored(&m, Attr::AttackDamage, "minecraft:effect.strength"),
        [(3.0, Op::AddValue)]
    );
    assert_eq!(m.attrs.attack_damage, 6.0);
}

/// Milk's `clear_all_effects` (`removeAllEffects`) and a
/// `remove_effects` component (`removeEffect` each) remove the modifiers of
/// what they remove.
#[test]
fn consumed_effect_removals_take_off_the_modifiers() {
    use crate::items::ConsumeEffect;
    let mut m = zombie();
    m.add_effect("minecraft:speed", -1, 0);
    m.add_effect("minecraft:weakness", -1, 0);
    assert_eq!(
        stored(&m, Attr::AttackDamage, "minecraft:effect.weakness"),
        [(-4.0, Op::AddValue)]
    );
    m.apply_consume_effect(&ConsumeEffect::RemoveEffects(vec![
        "minecraft:speed".into(),
    ]));
    assert!(stored(&m, Attr::MovementSpeed, SPEED).is_empty());
    assert_eq!(
        stored(&m, Attr::AttackDamage, "minecraft:effect.weakness").len(),
        1
    );
    m.apply_consume_effect(&ConsumeEffect::ClearAllEffects);
    assert!(m.extra.effects.is_empty());
    assert!(stored(&m, Attr::AttackDamage, "minecraft:effect.weakness").is_empty());
    assert_eq!(m.attrs.attack_damage, 3.0);
}

/// `MobEffect.addAttributeModifiers` skips an attribute the mob has no
/// instance of (`AttributeMap.getInstance` is null): a pig keeps no
/// Strength modifier, a zombie does; Slowness is `-0.15F` per level.
#[test]
fn only_the_attributes_a_mob_has_get_a_modifier() {
    let mut p = pig();
    p.add_effect("minecraft:strength", -1, 0);
    p.add_effect("minecraft:slowness", -1, 0);
    assert!(stored(&p, Attr::AttackDamage, "minecraft:effect.strength").is_empty());
    assert_eq!(
        stored(&p, Attr::MovementSpeed, "minecraft:effect.slowness"),
        [(-0.150_000_005_960_464_48, Op::AddMultipliedTotal)]
    );
    assert_eq!(
        finalize::value(&p, Attr::MovementSpeed),
        0.212_499_998_509_883_88
    );
    let mut z = zombie();
    z.add_effect("minecraft:strength", -1, 1);
    assert_eq!(
        stored(&z, Attr::AttackDamage, "minecraft:effect.strength"),
        [(6.0, Op::AddValue)]
    );
    assert_eq!(z.attrs.attack_damage, 9.0);
}

/// Health Boost: adding it raises the maximum and heals nothing
/// (`onEffectAdded` refreshes no attribute); its removal lowers the maximum
/// and `onAttributeUpdated(MAX_HEALTH)` lowers a health above it.
#[test]
fn health_boost_raises_the_maximum_and_its_end_clamps_the_health() {
    let mut m = zombie();
    m.add_effect("minecraft:health_boost", -1, 0);
    assert_eq!(
        stored(&m, Attr::MaxHealth, "minecraft:effect.health_boost"),
        [(4.0, Op::AddValue)]
    );
    assert_eq!(m.attrs.max_health, 24.0);
    assert_eq!(m.health, 20.0);
    m.health = 24.0;
    assert!(m.retain_effects(|_| false));
    assert!(stored(&m, Attr::MaxHealth, "minecraft:effect.health_boost").is_empty());
    assert_eq!(m.attrs.max_health, 20.0);
    assert_eq!(m.health, 20.0);
}

/// The saved `attributes` keep a given modifier across a save and a load,
/// beside the effect, and it still leaves with the effect afterwards.
#[test]
fn a_given_modifier_round_trips_through_the_save() {
    let mut m = pig();
    m.add_effect("minecraft:speed", -1, 1);
    let saved = mob_to_nbt(&m, [1, 0, 0, 1], None, None, |_| None, &Default::default());
    let Some(Tag::List(list)) = saved.get("attributes") else {
        panic!("no attributes: {saved:?}");
    };
    let speed = list
        .iter()
        .find(|a| a.get("id") == Some(&Tag::String("minecraft:movement_speed".into())))
        .expect("a movement speed entry");
    let Some(Tag::List(modifiers)) = speed.get("modifiers") else {
        panic!("no modifiers: {speed:?}");
    };
    assert_eq!(
        modifiers,
        &[Tag::compound([
            ("id", Tag::String(SPEED.into())),
            ("amount", Tag::Double(0.400_000_005_960_464_5f64.to_bits())),
            ("operation", Tag::String("add_multiplied_total".into())),
        ])]
    );
    let mut back = mob_from_nbt(&saved, &types(), 1, None).unwrap();
    assert!(back.has_effect("minecraft:speed"));
    assert_eq!(
        stored(&back, Attr::MovementSpeed, SPEED),
        [(0.400_000_005_960_464_5, Op::AddMultipliedTotal)]
    );
    assert_eq!(
        finalize::value(&back, Attr::MovementSpeed),
        0.350_000_001_490_116_1
    );
    assert!(back.retain_effects(|_| false));
    assert!(stored(&back, Attr::MovementSpeed, SPEED).is_empty());
    assert_eq!(finalize::value(&back, Attr::MovementSpeed), 0.25);
}

/// `ConversionType.convertCommon` copies each effect with `addEffect`: the
/// converted mob gets the modifier even when the old one had loaded the
/// effect without it.
#[test]
fn a_converted_mob_gets_the_modifier_of_each_copied_effect() {
    let tag = sunrise_core::nbt::parse_snbt(
        r#"{id:"minecraft:pig",Pos:[0.5d,64d,0.5d],active_effects:[{id:"minecraft:speed",amplifier:1b,duration:-1}]}"#,
    )
    .unwrap()
    .0;
    let from = mob_from_nbt(&tag, &types(), 1, None).unwrap();
    assert!(stored(&from, Attr::MovementSpeed, SPEED).is_empty());
    let mut to = zombie();
    crate::mobs::conversion::common(&from, &mut to, false);
    assert_eq!(
        stored(&to, Attr::MovementSpeed, SPEED),
        [(0.400_000_005_960_464_5, Op::AddMultipliedTotal)]
    );
}

/// A drinking witch's `-0.25` (`ADD_VALUE`) adds before Speed's
/// `ADD_MULTIPLIED_TOTAL` multiplies: (0.25 - 0.25) x 1.4 stands it still,
/// where taking 0.25 off the multiplied value would leave 0.1.
#[test]
fn a_drinking_witch_adds_its_modifier_before_the_effects_multiply() {
    let witch = mob_type(
        "minecraft:witch",
        Species::Witch,
        &[("minecraft:movement_speed", 0.25)],
    );
    let mut m = Mob::new(3, witch, Vec3::ZERO, 0.0);
    m.add_effect("minecraft:speed", -1, 1);
    assert_eq!(m.movement_speed(), 0.350_000_001_490_116_1);
    m.extra.flag = true;
    assert_eq!(m.movement_speed(), 0.0);
}

/// `PhantomMoveControl.speed` is the move control's own field: an effect's
/// modifier refreshing the movement speed attribute leaves it alone, and the
/// next move control step goes on from it (1.2 toward 1.8 by
/// `0.005 * 1.8 / 1.2`, in floats).
#[test]
fn a_phantoms_move_speed_is_not_the_movement_attribute() {
    let phantom = mob_type(
        "minecraft:phantom",
        Species::Phantom,
        &[("minecraft:movement_speed", 0.7)],
    );
    let mut m = Mob::new(4, phantom, Vec3::new(0.0, 64.0, 0.0), 0.0);
    m.extra.phantom_move_speed = 1.2;
    m.add_effect("minecraft:slowness", -1, 0);
    assert_eq!(m.extra.phantom_move_speed, 1.2);
    assert!(m.retain_effects(|_| false));
    assert_eq!(m.extra.phantom_move_speed, 1.2);
    m.add_effect("minecraft:speed", -1, 0);
    m.extra.vector = [0.0, 64.0, 10.0];
    crate::mobs::phantom::move_control(&mut m);
    assert_eq!(m.extra.phantom_move_speed, 1.207_500_1);
}

/// `AttributeInstance.calculateValue`: the baby's `ADD_MULTIPLIED_BASE`
/// sums with the base before Speed's `ADD_MULTIPLIED_TOTAL` multiplies:
/// (0.23 + 0.23 x 0.5) x (1 + 0.2F x 3) and
/// (0.35 + 0.35 x 0.2F) x (1 + 0.2F x 2).
#[test]
fn a_babys_bonus_sums_before_an_effect_multiplies() {
    let mut zombie = Mob::new(
        5,
        mob_type(
            "minecraft:zombie",
            Species::Zombie,
            &[("minecraft:movement_speed", 0.23)],
        ),
        Vec3::ZERO,
        0.0,
    );
    finalize::set_baby(&mut zombie, true);
    zombie.add_effect("minecraft:speed", -1, 2);
    assert_eq!(zombie.movement_speed(), 0.552_000_003_084_540_4);
    let mut piglin = Mob::new(
        6,
        mob_type(
            "minecraft:piglin",
            Species::Piglin,
            &[("minecraft:movement_speed", 0.35)],
        ),
        Vec3::ZERO,
        0.0,
    );
    finalize::set_baby(&mut piglin, true);
    piglin.add_effect("minecraft:speed", -1, 1);
    assert_eq!(piglin.movement_speed(), 0.588_000_003_963_708_9);
}

/// A cold strider's `-0.34F` (`ADD_MULTIPLIED_BASE`) sums with the base
/// before Speed II multiplies: (0.175 + 0.175 x -0.34F) x (1 + 0.2F x 2),
/// which multiplying the two in turn would round to ...534.
#[test]
fn a_striders_suffocation_sums_before_an_effect_multiplies() {
    let mut m = Mob::new(
        7,
        mob_type(
            "minecraft:strider",
            Species::Strider,
            &[("minecraft:movement_speed", 0.175)],
        ),
        Vec3::ZERO,
        0.0,
    );
    m.add_effect("minecraft:speed", -1, 1);
    m.extra.flag = true;
    assert_eq!(m.movement_speed(), 0.161_699_999_812_245_37);
}

/// `AbstractHorse.setOffspringAttribute` reads `getAttributeBaseValue`: a
/// horse given Speed II and Health Boost shows its partner its 0.3 and 20
/// bases, and its own roll is the one it would make without the effects.
#[test]
fn a_horses_offspring_inherit_its_bases_not_its_effects() {
    let horse = mob_type(
        "minecraft:horse",
        Species::Horse,
        &[
            ("minecraft:max_health", 20.0),
            ("minecraft:movement_speed", 0.3),
        ],
    );
    let items = crate::items::Items::empty();
    let mut plain = Mob::new(1, horse.clone(), Vec3::ZERO, 0.0);
    let mut boosted = Mob::new(1, horse, Vec3::ZERO, 0.0);
    boosted.add_effect("minecraft:speed", -1, 1);
    boosted.add_effect("minecraft:health_boost", -1, 0);
    let seen = boosted.seen(None, &items);
    assert_eq!(seen.stats[0], 20.0);
    assert_eq!(seen.stats[2], 0.3);
    let partner = plain.seen(None, &items);
    let want = crate::mobs::equine::OffspringAttributes::draw(&mut plain, &partner);
    let got = crate::mobs::equine::OffspringAttributes::draw(&mut boosted, &partner);
    assert_eq!(
        (got.max_health, got.jump, got.speed),
        (want.max_health, want.jump, want.speed)
    );
}

/// `AbstractHorse.canParent` of the partner compares its health with
/// `getMaxHealth`, Health Boost included: a tame partner with a base of 30
/// and Health Boost I (34) at 30 health is no mate, and is one at 34. Its
/// offspring roll still reads the base, 30.
#[test]
#[ignore = "daily"]
fn a_health_boosted_partner_below_its_maximum_is_no_mate() {
    use crate::mobs::goals::{Breed, Cx, Goal};
    let mut kind = None;
    assert!(
        crate::mobs::world::with_test_world(&[], |_, types| {
            kind = Some(Arc::new(types.types["minecraft:horse"].clone()));
        }),
        "the vanilla test resources must be available"
    );
    let kind = kind.unwrap();
    let horse = |id: u32, x: f64| {
        let mut m = Mob::new(id, kind.clone(), Vec3::new(x, 64.0, 0.5), 0.0);
        m.extra.tame = true;
        m.extra.in_love = 600;
        m.ground.on_ground = true;
        finalize::set_base(&mut m, Attr::MaxHealth, 30.0);
        m.health = 30.0;
        m
    };
    let mut a = horse(1, 0.5);
    let mut b = horse(2, 2.5);
    b.add_effect("minecraft:health_boost", -1, 0);
    assert_eq!(b.attrs.max_health, 34.0);
    let items = crate::items::Items::empty();
    let mut chosen = |partner: crate::mobs::Seen| {
        let mut events = Vec::new();
        let mut fixture = crate::health::TestLevel::new(0);
        let mut picked = false;
        assert!(crate::mobs::world::with_test_world(&[partner], |w, _| {
            let mut cx = Cx {
                w,
                level: &mut fixture.level(),
                events: &mut events,
            };
            picked = Breed::new(1.0).can_use(&mut a, &mut cx);
        }));
        picked
    };
    let below = b.seen(None, &items);
    assert_eq!(below.stats[0], 30.0);
    assert!(!chosen(below), "a partner below its boosted maximum");
    b.health = 34.0;
    assert!(chosen(b.seen(None, &items)), "the same partner at 34");
}

// ---- #1988: Jump Boost, Absorption and Invisibility ------------------------
//
// `MobEffects`: Jump Boost adds `minecraft:effect.jump_boost`, 1.0 per level,
// `add_value`, to `safe_fall_distance`; Absorption `minecraft:effect.absorption`,
// 4.0 per level, `add_value`, to `max_absorption`; Invisibility
// `minecraft:effect.waypoint_transmit_range_hide`, -1.0 per level,
// `add_multiplied_total`, to `waypoint_transmit_range`. Every living entity
// has the three attributes (`LivingEntity.createLivingAttributes`); a
// zombie's defaults are 3, 0 and 0.

const JUMP: &str = "minecraft:effect.jump_boost";
const ABSORPTION: &str = "minecraft:effect.absorption";
const HIDE: &str = "minecraft:effect.waypoint_transmit_range_hide";

/// The health a fall of `distance` blocks leaves a copy of a 20-health
/// zombie with (`causeFallDamage`:
/// `floor((distance + 1.0E-6 - safe_fall_distance) * 1 * 1)`); a copy, so no
/// earlier fall's hurt cooldown is on it.
fn health_after_fall(m: &Mob, distance: f64) -> f32 {
    let mut m = m.clone();
    m.health = 20.0;
    m.cause_fall_damage_from(
        crate::test_resources::server_loop_context(),
        &mut crate::health::TestLevel::new(0).level(),
        distance,
        1.0,
        &mut Vec::new(),
    );
    m.health
}

/// A NoAI zombie loaded with Jump Boost II, Absorption II and Invisibility
/// and a saved `waypoint_transmit_range` of 100 has none of the three
/// modifiers (`readAdditionalSaveData` adds none): safe fall distance 3, so
/// a 5.5-block fall hurts it for 2; max absorption 0; range 100.
#[test]
fn a_loaded_jump_boost_absorption_or_invisibility_adds_no_modifier() {
    let tag = sunrise_core::nbt::parse_snbt(
        r#"{id:"minecraft:zombie",Pos:[0.5d,64d,0.5d],NoAI:1b,attributes:[{id:"minecraft:waypoint_transmit_range",base:100d}],active_effects:[{id:"minecraft:jump_boost",amplifier:1b,duration:-1},{id:"minecraft:absorption",amplifier:1b,duration:-1},{id:"minecraft:invisibility",amplifier:0b,duration:-1}]}"#,
    )
    .unwrap()
    .0;
    let m = mob_from_nbt(&tag, &types(), 1, None).unwrap();
    assert!(m.has_effect("minecraft:jump_boost"));
    assert!(m.has_effect("minecraft:absorption"));
    assert!(m.has_effect("minecraft:invisibility"));
    assert!(stored(&m, Attr::SafeFallDistance, JUMP).is_empty());
    assert!(stored(&m, Attr::MaxAbsorption, ABSORPTION).is_empty());
    assert!(stored(&m, Attr::WaypointTransmitRange, HIDE).is_empty());
    assert_eq!(finalize::value(&m, Attr::SafeFallDistance), 3.0);
    assert_eq!(crate::mobs::armor::max_absorption(&m), 0.0);
    assert_eq!(finalize::value(&m, Attr::WaypointTransmitRange), 100.0);
    assert_eq!(health_after_fall(&m, 5.5), 18.0);
}

/// `/effect give <zombie> jump_boost infinite 1` stores 2.0 (`modifier value
/// get minecraft:effect.jump_boost`), so the safe fall distance reads 5 and
/// the same 5.5-block fall does no harm; the effect's end takes it off.
#[test]
fn a_given_jump_boost_raises_the_safe_fall_distance() {
    let mut m = zombie();
    assert_eq!(health_after_fall(&m, 5.5), 18.0);
    m.add_effect("minecraft:jump_boost", -1, 1);
    assert_eq!(
        stored(&m, Attr::SafeFallDistance, JUMP),
        [(2.0, Op::AddValue)]
    );
    assert_eq!(
        finalize::modifier_amount(&m, Attr::SafeFallDistance, JUMP),
        Some(2.0)
    );
    assert_eq!(finalize::value(&m, Attr::SafeFallDistance), 5.0);
    assert_eq!(finalize::base_value(&m, Attr::SafeFallDistance), 3.0);
    assert_eq!(health_after_fall(&m, 5.5), 20.0);
    assert_eq!(health_after_fall(&m, 7.5), 18.0);
    // `onEffectUpdated(true)`: Jump Boost III replaces the amount.
    m.add_effect("minecraft:jump_boost", -1, 2);
    assert_eq!(
        stored(&m, Attr::SafeFallDistance, JUMP),
        [(3.0, Op::AddValue)]
    );
    assert!(m.retain_effects(|_| false));
    assert!(stored(&m, Attr::SafeFallDistance, JUMP).is_empty());
    assert_eq!(finalize::value(&m, Attr::SafeFallDistance), 3.0);
    assert_eq!(health_after_fall(&m, 5.5), 18.0);
}

/// Absorption II stores 8.0 on `max_absorption` before `onEffectStarted`
/// raises the amount to 8 within that maximum; Absorption III replaces it
/// with 12.0 and raises the amount to 12; the effect's end takes the modifier
/// off and the amount is clamped to the maximum left, 0.
#[test]
fn a_given_absorption_raises_the_cap_and_its_end_lowers_it() {
    let mut m = zombie();
    m.add_effect("minecraft:absorption", -1, 1);
    assert_eq!(
        stored(&m, Attr::MaxAbsorption, ABSORPTION),
        [(8.0, Op::AddValue)]
    );
    assert_eq!(crate::mobs::armor::max_absorption(&m), 8.0);
    assert_eq!(m.absorption, 8.0);
    m.add_effect("minecraft:absorption", -1, 2);
    assert_eq!(
        stored(&m, Attr::MaxAbsorption, ABSORPTION),
        [(12.0, Op::AddValue)]
    );
    assert_eq!(crate::mobs::armor::max_absorption(&m), 12.0);
    assert_eq!(m.absorption, 12.0);
    assert!(m.retain_effects(|_| false));
    assert!(stored(&m, Attr::MaxAbsorption, ABSORPTION).is_empty());
    assert_eq!(crate::mobs::armor::max_absorption(&m), 0.0);
    assert_eq!(m.absorption, 0.0);
}

/// Invisibility's modifier has its own id, not `minecraft:effect.invisibility`:
/// on a zombie whose range was set to 100 it stores -1.0, which takes the
/// range to 100 x (1 - 1) = 0; its end gives the 100 back.
#[test]
fn invisibility_hides_the_waypoint_range_under_its_own_id() {
    let mut m = zombie();
    finalize::set_base(&mut m, Attr::WaypointTransmitRange, 100.0);
    m.add_effect("minecraft:invisibility", -1, 0);
    assert_eq!(
        stored(&m, Attr::WaypointTransmitRange, HIDE),
        [(-1.0, Op::AddMultipliedTotal)]
    );
    assert!(
        stored(
            &m,
            Attr::WaypointTransmitRange,
            "minecraft:effect.invisibility"
        )
        .is_empty()
    );
    assert_eq!(finalize::value(&m, Attr::WaypointTransmitRange), 0.0);
    assert!(m.retain_effects(|_| false));
    assert!(stored(&m, Attr::WaypointTransmitRange, HIDE).is_empty());
    assert_eq!(finalize::value(&m, Attr::WaypointTransmitRange), 100.0);
}

/// The three given modifiers are saved in their attributes' entries and a
/// load reads them back, so the reloaded zombie still has them.
#[test]
fn the_three_given_modifiers_round_trip_through_the_save() {
    let mut m = zombie();
    finalize::set_base(&mut m, Attr::WaypointTransmitRange, 100.0);
    m.add_effect("minecraft:jump_boost", -1, 1);
    m.add_effect("minecraft:absorption", -1, 1);
    m.add_effect("minecraft:invisibility", -1, 0);
    let saved = mob_to_nbt(&m, [1, 0, 0, 2], None, None, |_| None, &Default::default());
    let Some(Tag::List(list)) = saved.get("attributes") else {
        panic!("no attributes: {saved:?}");
    };
    let entry = |name: &str| {
        list.iter()
            .find(|a| a.get("id") == Some(&Tag::String(name.into())))
            .unwrap_or_else(|| panic!("no {name} entry: {list:?}"))
    };
    let modifier = |id: &str, amount: f64, op: &str| {
        Tag::List(vec![Tag::compound([
            ("id", Tag::String(id.into())),
            ("amount", Tag::Double(amount.to_bits())),
            ("operation", Tag::String(op.into())),
        ])])
    };
    assert_eq!(
        entry("minecraft:safe_fall_distance").get("modifiers"),
        Some(&modifier(JUMP, 2.0, "add_value"))
    );
    assert_eq!(
        entry("minecraft:max_absorption").get("modifiers"),
        Some(&modifier(ABSORPTION, 8.0, "add_value"))
    );
    let range = entry("minecraft:waypoint_transmit_range");
    assert_eq!(range.get("base"), Some(&Tag::Double(100f64.to_bits())));
    assert_eq!(
        range.get("modifiers"),
        Some(&modifier(HIDE, -1.0, "add_multiplied_total"))
    );
    let back = mob_from_nbt(&saved, &types(), 1, None).unwrap();
    assert_eq!(finalize::value(&back, Attr::SafeFallDistance), 5.0);
    assert_eq!(crate::mobs::armor::max_absorption(&back), 8.0);
    assert_eq!(
        finalize::base_value(&back, Attr::WaypointTransmitRange),
        100.0
    );
    assert_eq!(finalize::value(&back, Attr::WaypointTransmitRange), 0.0);
}

/// The issue named Haste, Mining Fatigue, Luck and Unluck too, but their
/// attributes (`attack_speed`, `luck`) are a player's only: on a mob
/// `addAttributeModifiers` finds no instance and adds nothing.
#[test]
fn haste_mining_fatigue_luck_and_unluck_store_nothing_on_a_mob() {
    let mut m = zombie();
    for effect in [
        "minecraft:haste",
        "minecraft:mining_fatigue",
        "minecraft:luck",
        "minecraft:unluck",
    ] {
        m.add_effect(effect, -1, 0);
    }
    assert_eq!(m.extra.effects.len(), 4);
    assert!(
        m.extra
            .attributes
            .as_ref()
            .is_none_or(|s| s.modifiers.is_empty()),
        "{:?}",
        m.extra.attributes
    );
}

/// A type built without an attribute row (a fixture, a remote client's
/// stand-in) reads its own `safe_fall_distance` field as the base: 6 for a
/// type whose field says 6.
#[test]
fn a_type_without_the_row_takes_its_own_safe_fall_distance() {
    let mut t = (*kind()).clone();
    t.attributes = Arc::default();
    t.safe_fall_distance = 6.0;
    let mut m = Mob::new(8, Arc::new(t), Vec3::ZERO, 0.0);
    assert_eq!(finalize::value(&m, Attr::SafeFallDistance), 6.0);
    // No instance in the type's row: `addAttributeModifiers` adds nothing.
    m.add_effect("minecraft:jump_boost", -1, 0);
    assert!(stored(&m, Attr::SafeFallDistance, JUMP).is_empty());
    assert_eq!(finalize::value(&m, Attr::SafeFallDistance), 6.0);
}

/// `removeAttributeModifiers` calls `AttributeInstance.removeModifier`,
/// which marks the instance dirty only when it held the id. A zombie that
/// loaded Absorption II with 8 absorption has no modifier, so `/effect clear`
/// runs no `onAttributeUpdated(MAX_ABSORPTION)` and the 8 stay; a given one
/// had the modifier, and its removal clamps the 10 it holds to the maximum
/// left, 0; removing another effect first clamps nothing.
#[test]
fn clearing_a_loaded_absorption_keeps_the_amount_and_a_given_one_clamps_it() {
    let tag = sunrise_core::nbt::parse_snbt(
        r#"{id:"minecraft:zombie",Pos:[0.5d,64d,0.5d],NoAI:1b,AbsorptionAmount:8f,active_effects:[{id:"minecraft:absorption",amplifier:1b,duration:-1}]}"#,
    )
    .unwrap()
    .0;
    let mut loaded = mob_from_nbt(&tag, &types(), 1, None).unwrap();
    assert_eq!(loaded.absorption, 8.0);
    assert!(loaded.retain_effects(|_| false));
    assert_eq!(loaded.absorption, 8.0);
    let mut given = zombie();
    given.add_effect("minecraft:absorption", -1, 1);
    given.add_effect("minecraft:speed", -1, 0);
    assert_eq!(given.absorption, 8.0);
    // Removing another effect leaves `max_absorption` clean: an amount above
    // the maximum (as a load sets it) stays.
    given.absorption = 10.0;
    assert!(given.retain_effects(|e| e.id != "minecraft:speed"));
    assert_eq!(given.absorption, 10.0);
    assert!(given.retain_effects(|_| false));
    assert_eq!(given.absorption, 0.0);
}

/// A zombie's live attribute map (what a `LivingRef` reads and the LAN sync
/// sends), built from its records over a catalog with the three attributes.
fn live_value(m: &Mob, name: &str) -> f64 {
    use sunrise_world::attribute_map::ValueKind;
    let bits = |v: f64| format!("{:016x}", v.to_bits());
    let row = |id: &str, default: f64, min: f64, max: f64| serde_json::json!({"id": id, "default": bits(default), "min": bits(min), "max": bits(max), "syncable": true});
    let catalog = Arc::new(
        sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "attributes": [
                row("minecraft:safe_fall_distance", 3.0, -1024.0, 1024.0),
                row("minecraft:max_absorption", 0.0, 0.0, 2048.0),
            ],
            "entities": {"minecraft:zombie": {
                "minecraft:safe_fall_distance": bits(3.0),
                "minecraft:max_absorption": bits(0.0),
            }}
        }))
        .unwrap(),
    );
    m.extra
        .live
        .read(
            &catalog,
            "minecraft:zombie",
            name,
            ValueKind::Computed,
            || {
                crate::entity_nbt::attribute_entries(m)
                    .into_iter()
                    .map(|(_, entry)| entry)
                    .collect()
            },
        )
        .unwrap()
}

/// A mod's transient `minecraft:effect.jump_boost` (5.0) on a zombie, then
/// Jump Boost II given: `addAttributeModifiers` removes the id wherever it is
/// and adds the permanent 2.0, so gameplay and the rebuilt live map both read
/// 3 + 2 = 5, not the old 5.0 replayed over it (10). Clearing the effect
/// leaves 3 in both.
#[test]
fn a_given_effect_replaces_a_mods_transient_modifier_with_its_id_everywhere() {
    let mut m = zombie();
    finalize::add_transient_modifier(&mut m, Attr::SafeFallDistance, JUMP, 5.0, Op::AddValue);
    assert_eq!(live_value(&m, "minecraft:safe_fall_distance"), 8.0);
    m.add_effect("minecraft:jump_boost", -1, 1);
    assert!(!finalize::has_transient_modifier(
        &m,
        Attr::SafeFallDistance,
        JUMP
    ));
    assert_eq!(finalize::value(&m, Attr::SafeFallDistance), 5.0);
    assert_eq!(live_value(&m, "minecraft:safe_fall_distance"), 5.0);
    // A transient Absorption id the given effect replaced stays gone through
    // the clear; another mod modifier stays.
    finalize::add_transient_modifier(&mut m, Attr::MaxAbsorption, ABSORPTION, 4.0, Op::AddValue);
    assert_eq!(live_value(&m, "minecraft:max_absorption"), 4.0);
    m.add_effect("minecraft:absorption", -1, 0);
    assert_eq!(live_value(&m, "minecraft:max_absorption"), 4.0);
    finalize::add_transient_modifier(
        &mut m,
        Attr::SafeFallDistance,
        "test:other",
        1.0,
        Op::AddValue,
    );
    assert!(m.retain_effects(|_| false));
    assert_eq!(finalize::value(&m, Attr::SafeFallDistance), 4.0);
    assert_eq!(live_value(&m, "minecraft:safe_fall_distance"), 4.0);
    assert_eq!(live_value(&m, "minecraft:max_absorption"), 0.0);
}

/// Clearing an Absorption effect that loaded without a modifier, while a
/// mod's transient `minecraft:effect.absorption` (8.0) is on `max_absorption`:
/// `removeModifier` finds that one, so the instance is dirty and the 8
/// absorption are clamped to the maximum left, 0; the live maximum reads 8
/// before and 0 after.
#[test]
fn clearing_absorption_takes_a_mods_transient_off_and_clamps() {
    let tag = sunrise_core::nbt::parse_snbt(
        r#"{id:"minecraft:zombie",Pos:[0.5d,64d,0.5d],NoAI:1b,AbsorptionAmount:8f,active_effects:[{id:"minecraft:absorption",amplifier:1b,duration:-1}]}"#,
    )
    .unwrap()
    .0;
    let mut m = mob_from_nbt(&tag, &types(), 1, None).unwrap();
    finalize::add_transient_modifier(&mut m, Attr::MaxAbsorption, ABSORPTION, 8.0, Op::AddValue);
    assert_eq!(crate::mobs::armor::max_absorption(&m), 8.0);
    assert_eq!(live_value(&m, "minecraft:max_absorption"), 8.0);
    assert!(m.retain_effects(|_| false));
    assert!(!finalize::has_transient_modifier(
        &m,
        Attr::MaxAbsorption,
        ABSORPTION
    ));
    assert_eq!(crate::mobs::armor::max_absorption(&m), 0.0);
    assert_eq!(live_value(&m, "minecraft:max_absorption"), 0.0);
    assert_eq!(m.absorption, 0.0);
}
