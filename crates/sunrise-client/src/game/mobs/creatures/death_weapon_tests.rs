use super::*;
use std::sync::{Arc, Mutex};

// Native mod registries are process-global. Each parallel test owns a fresh process
// and temporary working directory, while retaining the normal test runner parallelism.
pub(super) fn isolated() -> bool {
    const FLAG: &str = "SUNRISE_DEATH_WEAPON_CHILD";
    if std::env::var_os(FLAG).is_some() {
        return false;
    }
    let scratch = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap())
        .canonicalize()
        .unwrap();
    let thread = std::thread::current();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            thread.name().unwrap(),
            "--nocapture",
        ])
        .current_dir(scratch.path())
        .env(FLAG, "1")
        .env("SUNRISE_TEST_RESOURCES", resources)
        .env("SUNRISE_NO_AUDIO", "1")
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1");
    for key in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
    ] {
        command.env(key, scratch.path().join(key));
    }
    assert!(command.status().unwrap().success());
    true
}

pub(super) struct WeaponProbe {
    pub(super) expected: Option<mod_api::ItemStack>,
    pub(super) expected_source: Option<(&'static str, i32)>,
    pub(super) swap: Option<mod_api::ItemStack>,
    pub(super) seen: Arc<Mutex<usize>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for WeaponProbe {
    fn loot_drops(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LootDropsEvent<'_, '_>,
    ) {
        assert!(event.context.tool.is_none());
        if let Some(expected) = self.expected_source {
            let source = event.context.damage_source.unwrap();
            assert_eq!((source.damage_type, source.causing), expected);
        }
        assert_eq!(event.context.weapon_item(host), self.expected);
        if let Some(axe) = &self.swap {
            let source = event.context.damage_source.unwrap();
            let living = host
                .living(host.entity(source.causing as u32).unwrap())
                .unwrap();
            assert!(
                living
                    .set_hand_item(host, mod_api::Hand::Main, Some(axe.clone()))
                    .unwrap()
            );
            assert_eq!(event.context.weapon_item(host), Some(axe.clone()));
        }
        *self.seen.lock().unwrap() += 1;
    }
}

fn weapon_case(item: &str, enchantment: &str, level: u32, projectile: bool, swap: bool) {
    if isolated() {
        return;
    }
    crate::game::native_entity_refs::tests::with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.world_root = scratch.path().to_path_buf();
        game.save = Arc::new(sunrise_client::save::SaveDir::new(
            scratch.path().join("save"),
            sunrise_client::save::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        game.build_registries();
        game.next_entity = 1000;
        let loot = sunrise_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server_loot.set(Some(Arc::new(loot)));
        let ty = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut weapon = ItemStack::new(item, 1);
        weapon.enchant(enchantment, level);
        if item == "minecraft:bow" {
            weapon
                .data_mut()
                .set_loot_name("minecraft:custom_name", &serde_json::json!("Longbow"));
        }
        let expected = weapon.try_into_api().unwrap();
        let patch = expected.components().unwrap().as_nbt();
        assert_eq!(
            patch.get(&sunrise_core::nbt::JavaString::from(
                "minecraft:enchantments"
            )),
            Some(&mod_api::Tag::Compound(
                [(enchantment.into(), mod_api::Tag::Int(level as i32))].into()
            ))
        );
        if item == "minecraft:bow" {
            assert_eq!(
                patch.get(&sunrise_core::nbt::JavaString::from(
                    "minecraft:custom_name"
                )),
                Some(&mod_api::Tag::String("Longbow".into()))
            );
        }

        let axe = ItemStack::new("minecraft:diamond_axe", 1)
            .try_into_api()
            .unwrap();
        let seen = Arc::new(Mutex::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "weapon_probe",
            Box::new(WeaponProbe {
                expected: Some(expected),
                expected_source: None,
                swap: swap.then_some(axe),
                seen: seen.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let attacker_kind = match item {
                "minecraft:bow" => "minecraft:skeleton",
                "minecraft:crossbow" => "minecraft:pillager",
                "minecraft:trident" => "minecraft:drowned",
                _ => "minecraft:zombie",
            };
            let attacker_type = game.res.mob_types().get(attacker_kind).unwrap().clone();
            let mut attacker = Mob::new(41, attacker_type, Vec3::ZERO, 0.0);
            attacker.extra.set_main_hand_stack(Some(weapon.clone()));
            game.mobs.push(attacker);
            let mut victim = Mob::new(70, ty, Vec3::new(2.0, 64.0, 0.0), 0.0);
            let mut damage = Damage::of(if projectile {
                DamageKind::Arrow
            } else {
                DamageKind::MobAttack
            });
            damage.attacker = Some(Who::Mob(41));
            victim.health = 0.0;
            victim.last_damage = Some((damage, 0));
            if projectile {
                let trident = item == "minecraft:trident";
                let kind = if trident {
                    ProjectileKind::Trident
                } else {
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    }
                };
                let mut shot = Projectile::new(kind, Vec3::ZERO, Some(Who::Mob(41)));
                shot.id = 912;
                let arrow = shot.arrow.as_mut().unwrap();
                if trident {
                    arrow.pickup_item = weapon;
                } else {
                    arrow.fired_from_weapon = Some(weapon);
                }
                game.mobs
                    .iter_mut()
                    .find(|m| m.id == 41)
                    .unwrap()
                    .extra
                    .set_main_hand_stack(Some(ItemStack::new("minecraft:stick", 1)));
                // Commit through the same accepted-damage path as real projectile impacts.
                victim.health = 20.0;
                victim.last_damage = None;
                game.mobs.push(victim);
                damage.kind = game.projectile_damage_kind(&shot);
                assert!(game.damage_projectile_with_weapon(&shot, 70, 100.0, damage, None));
            } else {
                game.mob_die(&victim);
            }
        });
        assert_eq!(*seen.lock().unwrap(), 1);
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_a_longbow_power_ii_survives_hand_change() {
    crate::game::native_entity_refs::tests::run_with_game_stack(|| {
        weapon_case("minecraft:bow", "minecraft:power", 2, true, false);
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_b_crossbow_piercing_i() {
    crate::game::native_entity_refs::tests::run_with_game_stack(|| {
        weapon_case("minecraft:crossbow", "minecraft:piercing", 1, true, false);
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_c_trident_loyalty_ii() {
    crate::game::native_entity_refs::tests::run_with_game_stack(|| {
        weapon_case("minecraft:trident", "minecraft:loyalty", 2, true, false);
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_d_sharpness_v_sword_reads_live_axe() {
    crate::game::native_entity_refs::tests::run_with_game_stack(|| {
        weapon_case(
            "minecraft:diamond_sword",
            "minecraft:sharpness",
            5,
            false,
            true,
        );
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_e_fall_has_no_weapon() {
    if isolated() {
        return;
    }
    crate::game::native_entity_refs::tests::with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.world_root = scratch.path().to_path_buf();
        game.save = Arc::new(sunrise_client::save::SaveDir::new(
            scratch.path().join("save"),
            sunrise_client::save::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        game.build_registries();
        let loot = sunrise_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server_loot.set(Some(Arc::new(loot)));
        let seen = Arc::new(Mutex::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "fall_weapon_probe",
            Box::new(WeaponProbe {
                expected: None,
                expected_source: None,
                swap: None,
                seen: seen.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let ty = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut victim = Mob::new(70, ty, Vec3::ZERO, 0.0);
            victim.health = 0.0;
            victim.last_damage = Some((Damage::fall(), 0));
            game.mob_die(&victim);
        });
        assert_eq!(*seen.lock().unwrap(), 1);
    });
}

#[test]
#[ignore = "daily"]
fn a_player_credited_death_drops_experience_without_a_loot_snapshot() {
    if isolated() {
        return;
    }
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        game.next_entity = 1000;
        game.server_loot.set(None);
        game.orbs.clear();
        let pos = [2.0, 64.0, 0.0];
        let mut mob = Mob::new(
            70,
            game.res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone(),
            Vec3::new(pos[0], pos[1], pos[2]),
            0.0,
        );
        mob.health = 0.0;
        game.mobs.push(mob.clone());
        game.drop_death_loot(
            &mut mob,
            None,
            true,
            sunrise_world::loot::LootEntity::new("minecraft:player"),
            false,
            pos,
            None,
        );
        let total: i64 = game
            .orbs
            .iter()
            .map(|orb| i64::from(orb.value) * i64::from(orb.count))
            .sum();
        assert_eq!(total, 5, "an adult unequipped zombie's experience");
    });
}
