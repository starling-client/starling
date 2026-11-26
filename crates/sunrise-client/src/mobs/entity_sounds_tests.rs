use super::*;
use crate::mobs::{Damage, DamageKind, Mob, MobEvent, MobTypes, MobWorld};
use mod_api::Side;
use std::{path::PathBuf, sync::Arc};
use sunrise_core::geometry::Vec3;
use sunrise_mod_package::{Package, content::Content, enabled::EnabledMods};

fn fixture(template: &str, sounds: serde_json::Value, run: impl FnOnce(&MobWorld, &MobTypes)) {
    let base = std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3")
        });
    if !base.join("entity-types.json").exists() {
        assert!(
            std::env::var_os("SUNRISE_TEST_RESOURCES").is_none(),
            "missing requested resources"
        );
        eprintln!("skipped: no prepared resources for entity sounds");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bird.smod");
    let manifest = "id = \"bird\"\nname = \"Bird\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    let declaration = serde_json::json!({"template":template,"sounds":sounds}).to_string();
    std::fs::write(
        &path,
        Package::write_files(
            manifest,
            &[(
                "data/bird/sunrise/entity_type/vulture.json",
                declaration.as_bytes(),
            )],
        )
        .unwrap(),
    )
    .unwrap();
    let package = Package::read(&path).unwrap();
    let mods = [crate::mod_content::ContentMod {
        id: "bird".into(),
        content: Content::read(&path, &package.manifest).unwrap(),
        jars: vec![],
    }];
    let inventory = EnabledMods::new([("bird".into(), Side::Both)]);
    let root = crate::mod_content::resource_root(&base, dir.path(), &mods, dir.path(), &inventory)
        .unwrap();
    assert!(crate::mobs::world::with_resource_test_world(
        &root,
        &[],
        true,
        true,
        |world, registry| {
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            world
                .set(0, 0, 0, registry.default_state("minecraft:stone").unwrap())
                .unwrap();
        },
        run
    ));
}
fn mob(types: &MobTypes, id: &str) -> Mob {
    let mut mob = Mob::new(
        1,
        Arc::new(types.types[id].clone()),
        Vec3::new(0.5, 1.0, 0.5),
        0.0,
    );
    mob.ground.on_ground = true;
    mob.ground.supporting_block = Some(sunrise_core::position::BlockPos::new(0, 0, 0));
    mob
}
// Use the production audio capture consumed by server packet delivery, then replay
// the captured ids through seeded client audio. Muted slots reach neither sink.
fn ids(events: &[MobEvent]) -> Vec<String> {
    let source = sunrise_assets::resource_source::ResourceSource::directory(PathBuf::from(
        "/nonexistent/sunrise-content-entity-sounds",
    ));
    let mut server = crate::audio::Audio::silent(&source);
    let mut remote = crate::audio::Audio::silent(&source);
    server.capture = Some(vec![]);
    for event in events {
        match event {
            MobEvent::Sound(id, at, volume, pitch) => {
                server.play(&full_id(id), Some(*at), *volume, *pitch)
            }
            MobEvent::SoundId(id, at, volume, pitch) => server.play(id, Some(*at), *volume, *pitch),
            _ => {}
        }
    }
    let captured = server.capture.take().unwrap();
    for sound in &captured {
        remote.play_seeded(
            &sound.event,
            Some(sound.position),
            sound.volume,
            sound.pitch,
            42,
        );
    }
    let ids: Vec<_> = captured.into_iter().map(|sound| sound.event).collect();
    assert_eq!(remote.heard.values().sum::<u32>() as usize, ids.len());
    assert_eq!(server.heard, remote.heard);
    assert!(ids.iter().all(|id| !id.contains("minecraft:minecraft:")));
    ids
}
fn ambient(m: &mut Mob, w: &MobWorld) -> Vec<String> {
    let mut events = vec![];
    crate::mobs::ai::ambient_sound(m, w, &mut events);
    ids(&events)
}
fn steps(m: &mut Mob, w: &MobWorld) -> Vec<String> {
    let mut events = vec![];
    m.movement_emission(w, Vec3::new(2.0, 0.0, 0.0), &mut events);
    ids(&events)
}
fn hit(m: &mut Mob, amount: f32) -> Vec<String> {
    let mut fixture = crate::health::TestLevel::new(0);
    assert!(m.damage(
        &mut fixture.level(),
        amount,
        Damage::of(DamageKind::Generic)
    ));
    ids(&std::mem::take(&mut m.outbox))
}

#[test]
fn entity_sounds_inherit_template_ambient_and_stone_footsteps() {
    fixture(
        "minecraft:wandering_trader",
        serde_json::json!({}),
        |w, types| {
            let mut m = mob(types, "bird:vulture");
            assert_eq!(
                ambient(&mut m, w),
                ["minecraft:entity.wandering_trader.ambient"]
            );
        },
    );
    fixture("minecraft:villager", serde_json::json!({}), |w, types| {
        let mut template = mob(types, "minecraft:villager");
        let mut content = mob(types, "bird:vulture");
        assert_eq!(steps(&mut template, w), ["minecraft:block.stone.step"]);
        assert_eq!(steps(&mut content, w), ["minecraft:block.stone.step"]);
        content.extra.flag2 = true;
        template.extra.flag2 = true;
        assert_eq!(
            ambient(&mut content, w),
            ["minecraft:entity.villager.trade"]
        );
        assert_eq!(
            ambient(&mut template, w),
            ["minecraft:entity.villager.trade"]
        );
    });
}
#[test]
fn entity_sounds_override_each_trigger_with_exact_full_ids() {
    fixture(
        "minecraft:wandering_trader",
        serde_json::json!({"ambient":"bird:entity.vulture.ambient","hurt":"bird:hurt","death":"bird:death","step":"bird:step"}),
        |w, types| {
            assert_eq!(
                ambient(&mut mob(types, "bird:vulture"), w),
                ["bird:entity.vulture.ambient"]
            );
            assert_eq!(hit(&mut mob(types, "bird:vulture"), 1.0), ["bird:hurt"]);
            assert_eq!(hit(&mut mob(types, "bird:vulture"), 1000.0), ["bird:death"]);
            assert_eq!(steps(&mut mob(types, "bird:vulture"), w), ["bird:step"]);
        },
    );
}
#[test]
fn entity_sounds_mute_all_four_without_emitting_sound_events() {
    fixture(
        "minecraft:villager",
        serde_json::json!({"ambient":"none","hurt":"none","death":"none","step":"none"}),
        |w, types| {
            assert!(ambient(&mut mob(types, "bird:vulture"), w).is_empty());
            assert!(hit(&mut mob(types, "bird:vulture"), 1.0).is_empty());
            assert!(hit(&mut mob(types, "bird:vulture"), 1000.0).is_empty());
            assert!(steps(&mut mob(types, "bird:vulture"), w).is_empty());
        },
    );
}
#[test]
fn entity_sounds_partial_override_keeps_other_template_slots() {
    fixture(
        "minecraft:wandering_trader",
        serde_json::json!({"hurt":"bird:hurt"}),
        |w, types| {
            assert_eq!(
                ambient(&mut mob(types, "bird:vulture"), w),
                ["minecraft:entity.wandering_trader.ambient"]
            );
            assert_eq!(hit(&mut mob(types, "bird:vulture"), 1.0), ["bird:hurt"]);
            assert_eq!(
                hit(&mut mob(types, "bird:vulture"), 1000.0),
                ["minecraft:entity.wandering_trader.death"]
            );
            assert_eq!(
                steps(&mut mob(types, "bird:vulture"), w),
                ["minecraft:block.stone.step"]
            );
        },
    );
}
#[test]
fn entity_sounds_explicit_minecraft_id_is_not_prefixed_twice() {
    fixture(
        "minecraft:villager",
        serde_json::json!({"ambient":"minecraft:entity.wandering_trader.ambient"}),
        |w, types| {
            assert_eq!(
                ambient(&mut mob(types, "bird:vulture"), w),
                ["minecraft:entity.wandering_trader.ambient"]
            );
        },
    );
}

#[test]
fn entity_sounds_step_only_override_and_mute_preserve_ambient() {
    for step in ["bird:step", "none"] {
        fixture(
            "minecraft:villager",
            serde_json::json!({"step":step}),
            |w, types| {
                assert_eq!(
                    ambient(&mut mob(types, "bird:vulture"), w),
                    ["minecraft:entity.villager.ambient"]
                );
                let heard = steps(&mut mob(types, "bird:vulture"), w);
                if step == "none" {
                    assert!(heard.is_empty());
                } else {
                    assert_eq!(heard, ["bird:step"]);
                }
            },
        );
    }
}

#[test]
fn entity_sounds_malformed_row_reports_entity_id() {
    for sounds in [serde_json::json!({"hurt":42}), serde_json::Value::Null] {
        let row = serde_json::json!({"sounds":sounds});
        let error = Sounds::load("bird:vulture", &row).unwrap_err().to_string();
        assert!(error.contains("bird:vulture"), "{error}");
        let dir = tempfile::tempdir().unwrap();
        let mut row = row;
        row["template"] = serde_json::json!("minecraft:villager");
        std::fs::write(
            dir.path().join("entity-types.json"),
            serde_json::to_vec(&serde_json::json!({"bird:vulture":row})).unwrap(),
        )
        .unwrap();
        let attributes =
            sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
                "data_version": sunrise_world::DATA_VERSION,
                "attributes": [],
                "entities": {},
            }))
            .unwrap();
        let Err(error) = MobTypes::load_for_server(dir.path(), &std::sync::Arc::new(attributes))
        else {
            panic!("malformed sounds were admitted by the entity loader");
        };
        assert!(error.to_string().contains("bird:vulture"), "{error}");
    }
    assert!(Sounds::load("bird:vulture", &serde_json::json!({})).is_ok());
}

/// The played sounds with their volume and pitch, as the server sends them.
fn played(events: &[MobEvent]) -> Vec<(String, f32, f32)> {
    events
        .iter()
        .filter_map(|event| match event {
            MobEvent::Sound(id, _, volume, pitch) => {
                Some((full_id(id).into_owned(), *volume, *pitch))
            }
            MobEvent::SoundId(id, _, volume, pitch) => Some((id.clone(), *volume, *pitch)),
            _ => None,
        })
        .collect()
}
#[test]
fn entity_sounds_declared_step_plays_at_its_volume_and_pitch() {
    let mut sounds = Sounds::default();
    // An omitted step is the template's, from the block under the mob.
    assert_eq!(declared_step(&sounds), None);
    sounds.step = Selection::Mute;
    assert_eq!(declared_step(&sounds), Some(None));
    // The plain id keeps the block step's `soundType.getVolume() * 0.15F` and pitch.
    sounds.step = Selection::Id("bird:step");
    assert_eq!(
        declared_step(&sounds),
        Some(Some((Cow::Borrowed("bird:step"), 0.15, 1.0)))
    );
    // An override of `playStepSound` calling `playSound(step, volume, pitch)`.
    sounds.step_volume_pitch = Some((1.0, 0.8));
    assert_eq!(
        declared_step(&sounds),
        Some(Some((Cow::Borrowed("bird:step"), 1.0, 0.8)))
    );
}

#[test]
fn entity_sounds_declared_volume_and_voice_pitch_reach_ambient_hurt_and_death() {
    use sunrise_core::random::{LegacyRandom, RandomSource};
    let declared = |sounds| {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.sound_overrides = sounds;
        Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0)
    };
    // `Mob.playAmbientSound` is `makeSound(getAmbientSound())`: `getSoundVolume`, `getVoicePitch`.
    let ambient = |m: &mut Mob| {
        m.random = LegacyRandom::new(7);
        let mut events = vec![];
        m.voice("bird:ambient", &mut events);
        (played(&events), m.random.next_int())
    };
    let sounds = Sounds {
        hurt: Selection::Id("bird:hurt"),
        death: Selection::Id("bird:death"),
        volume: Some(0.25),
        voice_pitch: Some(0.5),
        ..Sounds::default()
    };
    // Java Random(7): spread from two nextFloat() draws, then nextInt().
    let expected = |volume, pitch| {
        (
            vec![("bird:ambient".to_owned(), volume, f32::from_bits(pitch))],
            0xbfc9_945a_u32 as i32,
        )
    };
    assert_eq!(
        ambient(&mut declared(sounds.clone())),
        expected(0.25, 0x3f04_b7fa)
    );
    // A baby's raised pitch is the base times vanilla's 1.5.
    let mut baby = declared(sounds.clone());
    baby.extra.age = -24000;
    assert_eq!(ambient(&mut baby), expected(0.25, 0x3f44_b7fa));
    // `playHurtSound` and the death sound, from real hits.
    for (amount, id) in [(1.0, "bird:hurt"), (1000.0, "bird:death")] {
        let mut m = declared(sounds.clone());
        let mut level = crate::health::TestLevel::new(0);
        assert!(m.damage(&mut level.level(), amount, Damage::of(DamageKind::Generic)));
        let heard = played(&std::mem::take(&mut m.outbox));
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!((heard[0].0.as_str(), heard[0].1), (id, 0.25));
    }
    // Omitted keys keep the template's: a cow's `getSoundVolume` 0.4 and a pitch around 1.0.
    assert_eq!(
        ambient(&mut declared(Sounds::default())),
        expected(0.4, 0x3f82_5bfd)
    );
}

#[test]
fn entity_sounds_any_declared_interval_negates_and_multiplies_as_java_ints() {
    for interval in [i32::MIN, -5, 0, i32::MAX] {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.ambient_interval = interval;
        // A hurt mob resets its ambient clock to `-getAmbientSoundInterval()`.
        let mut m = Mob::new(1, Arc::new(kind.clone()), Vec3::ZERO, 0.0);
        let mut level = crate::health::TestLevel::new(0);
        assert!(m.damage(&mut level.level(), 1.0, Damage::of(DamageKind::Generic)));
        assert_eq!(m.ambient_timer, interval.wrapping_neg());
        // HappyGhast.getAmbientSoundInterval: `super` times 6 while ridden.
        kind.species = crate::mobs::Species::HappyGhast;
        let mut ghast = Mob::new(2, Arc::new(kind), Vec3::ZERO, 0.0);
        ghast.extra.rider = Some(3);
        assert_eq!(ghast.ambient_interval(), interval.wrapping_mul(6));
    }
}

/// Runs `test` in a child process of its own: loading mob types fills process-wide
/// tables (the sulfur cube's item lookup), which no other test may see filled.
fn in_own_process(test: &str) -> bool {
    const CHILD: &str = "SUNRISE_TEST_ENTITY_SOUNDS_CHILD";
    let module = module_path!().split_once("::").unwrap().1;
    let name = format!("{module}::{test}");
    if std::env::var_os(CHILD).is_some_and(|value| value == name.as_str()) {
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([&name, "--exact", "--ignored", "--test-threads=1"])
        .env(CHILD, &name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("test result: ok. 1 passed;"),
        "{name} failed in its own process:
{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

#[test]
#[ignore = "daily"]
fn entity_sounds_declared_interval_replaces_the_templates_at_load() {
    if !in_own_process("entity_sounds_declared_interval_replaces_the_templates_at_load") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let rows = serde_json::json!({
        "bird:vulture": {"template":"minecraft:villager","sounds":{
            "ambient_interval":200,
            "step":{"sound":"bird:step","volume":0.625,"pitch":0.875},
        }},
        "bird:quiet": {"template":"minecraft:villager","sounds":{"ambient_interval":i32::MIN}},
        "bird:plain": {"template":"minecraft:villager"},
    });
    std::fs::write(
        dir.path().join("entity-types.json"),
        serde_json::to_vec(&rows).unwrap(),
    )
    .unwrap();
    let attributes = sunrise_world::attributes::entity::Catalog::from_json(&serde_json::json!({
        "data_version": sunrise_world::DATA_VERSION,
        "attributes": [],
        "entities": {},
    }))
    .unwrap();
    let types = MobTypes::load_for_server(dir.path(), &std::sync::Arc::new(attributes)).unwrap();
    let vulture = &types.types["bird:vulture"];
    assert_eq!(
        declared_step(&vulture.sound_overrides),
        Some(Some((Cow::Borrowed("bird:step"), 0.625, 0.875)))
    );
    let interval = |id: &str| types.types[id].ambient_interval;
    assert_eq!(interval("bird:vulture"), 200);
    assert_eq!(interval("bird:quiet"), i32::MIN);
    // Omitted: the villager's `Mob.getAmbientSoundInterval`, 80.
    assert_eq!(interval("bird:plain"), 80);
}
