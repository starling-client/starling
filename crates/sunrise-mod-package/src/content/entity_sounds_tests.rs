use super::*;
use crate::{Package, content::Content};

#[test]
fn entity_sounds_admission_preserves_inherit_id_and_mute_and_rejects_bad_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bird.smod");
    let manifest = "id = \"bird\"\nname = \"Bird\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    for slot in ["ambient", "hurt", "death", "step"] {
        let file = format!(
            r#"{{"template":"minecraft:wandering_trader","sounds":{{"{slot}":"bird:bad id"}}}}"#
        );
        std::fs::write(
            &path,
            Package::write_files(
                manifest,
                &[(
                    "data/bird/sunrise/entity_type/vulture.json",
                    file.as_bytes(),
                )],
            )
            .unwrap(),
        )
        .unwrap();
        let package = Package::read(&path).unwrap();
        let error = Content::read(&path, &package.manifest).expect_err("bad sound admitted");
        assert!(
            error.contains("bird:vulture") && error.contains(slot),
            "{error}"
        );
    }
    let sounds: EntitySounds =
        serde_json::from_str(r#"{"hurt":"bird:hurt","step":"none"}"#).unwrap();
    assert_eq!(sounds.ambient, None);
    assert_eq!(sounds.hurt, Some(SoundOverride::Id("bird:hurt".into())));
    assert!(matches!(sounds.step, Some(StepOverride::Mute(_))));
    sounds.validate("bird:vulture").unwrap();
    for bad in ["hurt", "Bird:hurt", "bird:", ":hurt", "bird:a:b"] {
        let sounds = EntitySounds {
            hurt: Some(SoundOverride::Id(bad.into())),
            ..Default::default()
        };
        assert!(sounds.validate("bird:vulture").is_err(), "{bad}");
    }
}

#[test]
fn entity_sounds_thrown_type_refuses_sounds_with_exact_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bird.smod");
    let manifest = "id = \"bird\"\nname = \"Bird\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    let declaration = br#"{"template":"minecraft:snowball","sounds":{"hurt":"bird:hurt"}}"#;
    std::fs::write(
        &path,
        Package::write_files(
            manifest,
            &[("data/bird/sunrise/entity_type/bolt.json", declaration)],
        )
        .unwrap(),
    )
    .unwrap();
    let package = Package::read(&path).unwrap();
    let error = Content::read(&path, &package.manifest).expect_err("thrown sounds admitted");
    assert_eq!(
        error,
        "data/bird/sunrise/entity_type/bolt.json: only a living type has sounds (its template is minecraft:snowball)"
    );
}

#[test]
fn entity_sounds_numbers_admit_any_finite_float_and_any_interval() {
    let sounds: EntitySounds = serde_json::from_str(
        r#"{"step":{"sound":"bird:step","volume":1.0,"pitch":0.8},"volume":-0.5,"voice_pitch":0.0,"ambient_interval":-2147483648}"#,
    )
    .unwrap();
    assert_eq!(
        sounds.step,
        Some(StepOverride::Sound(StepSound {
            sound: "bird:step".into(),
            volume: 1.0,
            pitch: 0.8,
        }))
    );
    assert_eq!(sounds.volume, Some(-0.5));
    assert_eq!(sounds.voice_pitch, Some(0.0));
    // Vanilla bounds none of these, so neither does the declaration.
    assert_eq!(sounds.ambient_interval, Some(i32::MIN));
    sounds.validate("bird:vulture").unwrap();
    for interval in [0, -80, i32::MAX] {
        let sounds: EntitySounds =
            serde_json::from_str(&format!(r#"{{"ambient_interval":{interval}}}"#)).unwrap();
        assert_eq!(sounds.ambient_interval, Some(interval));
    }
    // A plain id and "none" keep their meanings.
    let plain: EntitySounds = serde_json::from_str(r#"{"step":"bird:step"}"#).unwrap();
    assert_eq!(plain.step, Some(StepOverride::Id("bird:step".into())));
    let mute: EntitySounds = serde_json::from_str(r#"{"step":"none"}"#).unwrap();
    assert_eq!(mute.step, Some(StepOverride::Mute(Mute::None)));
    // The object's id is checked as a plain one is.
    let bad: EntitySounds =
        serde_json::from_str(r#"{"step":{"sound":"step","volume":1.0,"pitch":1.0}}"#).unwrap();
    let error = bad.validate("bird:vulture").unwrap_err();
    assert!(error.contains("sounds.step"), "{error}");
}

fn admit(sounds: &str) -> Result<EntitySounds, String> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bird.smod");
    let manifest = "id = \"bird\"
name = \"Bird\"
version = \"1.0\"
api = \"0.2\"
side = \"both\"
capabilities = [\"registries\"]
";
    let file = format!(r#"{{"template":"minecraft:wandering_trader","sounds":{sounds}}}"#);
    std::fs::write(
        &path,
        Package::write_files(
            manifest,
            &[(
                "data/bird/sunrise/entity_type/vulture.json",
                file.as_bytes(),
            )],
        )
        .unwrap(),
    )
    .unwrap();
    let package = Package::read(&path).unwrap();
    let mut content = Content::read(&path, &package.manifest)?;
    Ok(content.entity_types.pop().unwrap().sounds)
}

#[test]
fn entity_sounds_admission_rounds_numbers_once_to_f32() {
    // The decimal just above the midpoint and the negative integer outside i64 both land
    // exactly on a midpoint if rounded to f64 first. The bits below are from Float.parseFloat.
    for (number, bits) in [
        ("1.0000000596046448", 0x3f80_0001),
        ("-9223372586610589697", 0xdf00_0001),
    ] {
        let sounds = admit(&format!(
            r#"{{"volume":{number},"voice_pitch":{number},"step":{{"sound":"bird:step","volume":{number},"pitch":{number}}}}}"#,
        ))
        .unwrap();
        let Some(StepOverride::Sound(step)) = sounds.step else {
            panic!("missing step sound");
        };
        assert_eq!(
            [
                sounds.volume.unwrap(),
                sounds.voice_pitch.unwrap(),
                step.volume,
                step.pitch
            ]
            .map(f32::to_bits),
            [bits; 4],
            "{number}",
        );
    }
}

/// Refused at admission, naming the file and the key: overflowing numbers, a duplicate
/// scalar, the step's positional array or a numeric step, and unknown keys.
#[test]
fn entity_sounds_admission_refuses_bad_numbers_and_step_forms_by_key() {
    for (sounds, key) in [
        (r#"{"volume":1e39}"#, "sounds.volume"),
        (r#"{"volume":1e400}"#, "sounds.volume"),
        (r#"{"voice_pitch":-1e39}"#, "sounds.voice_pitch"),
        (r#"{"voice_pitch":1e400}"#, "sounds.voice_pitch"),
        (
            r#"{"step":{"sound":"bird:step","volume":1e39,"pitch":1.0}}"#,
            "sounds.step.volume",
        ),
        (
            r#"{"step":{"sound":"bird:step","volume":1.0,"pitch":-1e39}}"#,
            "sounds.step.pitch",
        ),
        (
            r#"{"step":{"sound":"bird:step","volume":1e400,"pitch":1.0}}"#,
            "sounds.step.volume",
        ),
        (
            r#"{"step":{"sound":"bird:step","volume":1e39,"volume":0.5,"pitch":1.0}}"#,
            "sounds.step.volume",
        ),
        (
            r#"{"step":{"sound":"bird:step","volume":0.5,"volume":0.5,"pitch":1.0}}"#,
            "duplicate field `volume`",
        ),
        (r#"{"step":["bird:step",1.0,0.8]}"#, "sounds.step"),
        (r#"{"step":1e400}"#, "sounds.step"),
        (
            r#"{"step":{"sound":"bird:step","volume":1.0,"pitch":1.0,"range":16}}"#,
            "range",
        ),
    ] {
        let error = admit(sounds).expect_err(sounds);
        assert!(
            error.starts_with("data/bird/sunrise/entity_type/vulture.json: ")
                && error.contains(key),
            "{sounds}: {error}"
        );
    }
}
