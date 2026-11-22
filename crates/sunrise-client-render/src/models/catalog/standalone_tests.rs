//! CPU bake proofs: literal positions/UVs, dependency atomicity and fresh resource loads.
use super::*;
use crate::models::{StandaloneModels, freeze_standalone, with_test_registrations};
use mod_api::StandaloneModelsEvent;

struct Fixture(tempfile::TempDir);
impl Fixture {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn write(&self, name: &str, text: &str) {
        let path = self
            .0
            .path()
            .join(format!("assets/source/models/{name}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn north_arrow_texture(&self) {
        let path = self.0.path().join("assets/source/textures/north_arrow.png");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..16usize {
            for x in 0..16usize {
                let shaft = (7..=8).contains(&x) && (5..=14).contains(&y);
                let head = (1..=7).contains(&y) && x.abs_diff(7) < y;
                if shaft || head {
                    pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4]
                        .copy_from_slice(&[255, 0, 0, 255]);
                }
            }
        }
        let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    fn bake(&self, standalone: &StandaloneModels) -> StateModels {
        let source = crate::resource_source::ResourceSource::directory(self.0.path().into());
        let parsed = crate::model_library::ParsedModels::load(&source);
        let mut sprites = Sprites::new(source.clone());
        let mut catalog = StateModels::default();
        catalog
            .bake_standalone(&source, &mut sprites, &parsed, &standalone.names)
            .unwrap();
        catalog.bake_standalone_bakers(&source, &mut sprites, &parsed, &standalone.bakers);
        catalog
    }
}
fn rotated(event: &mut StandaloneModelsEvent<'_>, id: &str, turns: [u8; 3], uvlock: bool) {
    let arrow = "source:arrow";
    event.register(id, [arrow], move |bake| {
        bake.append_dependency(arrow, turns, uvlock)
    });
}
fn close<const N: usize>(actual: [[f32; N]; 4], expected: [[f32; N]; 4]) {
    for (actual, expected) in actual
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        assert!(
            (actual - expected).abs() < 0.00001,
            "{actual} != {expected}"
        );
    }
}
fn face(model: &BakedModel, facing: Direction) -> &ModelQuad {
    &model
        .faces
        .iter()
        .find(|face| face.quad.facing == facing)
        .unwrap()
        .quad
}

#[test]
fn standalone_rotated_arrow_has_literal_geometry_and_world_locked_uvs() {
    let fixture = Fixture::new();
    fixture.north_arrow_texture();
    // A north-pointing arrow on the top and an asymmetric UV rectangle on the north wall.
    fixture.write("arrow", r##"{"textures":{"arrow":"source:north_arrow"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#arrow","uv":[2,4,10,12]},"up":{"texture":"#arrow","uv":[0,0,16,16]}}}]}"##);
    let models = with_test_registrations(
        |event| {
            rotated(event, "result:east", [0, 1, 0], false);
            rotated(event, "result:locked", [0, 1, 0], true);
            rotated(event, "result:x", [1, 0, 0], false);
            rotated(event, "result:z", [0, 0, 1], false);
            rotated(event, "result:wrapped", [0, 5, 0], false);
        },
        |staged| fixture.bake(&freeze_standalone(staged)),
    );
    let east = models.standalone("result:east").unwrap();
    let locked = models.standalone("result:locked").unwrap();
    let east_positions = [[1., 1., 1.], [1., 0., 1.], [1., 0., 0.], [1., 1., 0.]];
    close(face(east, Direction::East).positions, east_positions);
    close(face(locked, Direction::East).positions, east_positions);
    // A vertical north wall stays upright under a Y turn, with or without UV locking.
    // The top arrow is where north -> east really rotates texture orientation.
    let wall_uv = [[0.125, 0.25], [0.125, 0.75], [0.625, 0.75], [0.625, 0.25]];
    close(face(east, Direction::East).uv, wall_uv);
    close(face(locked, Direction::East).uv, wall_uv);
    let top_positions = [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]];
    close(face(east, Direction::Up).positions, top_positions);
    close(face(locked, Direction::Up).positions, top_positions);
    close(
        face(east, Direction::Up).uv,
        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
    );
    close(
        face(locked, Direction::Up).uv,
        [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
    );
    close(
        face(models.standalone("result:x").unwrap(), Direction::Down).positions,
        [[0., 0., 1.], [0., 0., 0.], [1., 0., 0.], [1., 0., 1.]],
    );
    close(
        face(models.standalone("result:x").unwrap(), Direction::Down).uv,
        [[0.625, 0.75], [0.625, 0.25], [0.125, 0.25], [0.125, 0.75]],
    );
    close(
        face(models.standalone("result:z").unwrap(), Direction::East).positions,
        east_positions,
    );
    close(
        face(models.standalone("result:z").unwrap(), Direction::East).uv,
        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
    );
    close(
        face(models.standalone("result:wrapped").unwrap(), Direction::Up).uv,
        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
    );
    assert!(
        models.standalone("source:arrow").is_none(),
        "a dependency is not a registration"
    );
}

fn cube(from: [u8; 3], to: [u8; 3], texture: &str) -> String {
    serde_json::json!({"textures":{"all":texture}, "elements":[{
    "from":from, "to":to, "faces":{
        "down":{"texture":"#all"}, "up":{"texture":"#all"},
        "north":{"texture":"#all"}, "south":{"texture":"#all"},
        "west":{"texture":"#all"}, "east":{"texture":"#all"}
    }}]})
    .to_string()
}
fn combined(event: &mut StandaloneModelsEvent<'_>, parts: &'static [&'static str]) {
    event.register(
        "different_namespace:assembly",
        parts.iter().copied(),
        move |bake| {
            for part in parts {
                bake.append_dependency(part, [0; 3], false)?;
            }
            Ok(())
        },
    );
}

#[test]
fn standalone_combined_keeps_both_cubes_and_rebakes_changed_dependencies() {
    let fixture = Fixture::new();
    fixture.write("base", &cube([0, 0, 0], [8, 8, 8], "source:base"));
    fixture.write("overlay", &cube([8, 8, 8], [16, 16, 16], "source:overlay"));
    let (first, second) = with_test_registrations(
        |event| combined(event, &["source:base", "source:overlay"]),
        |staged| {
            let standalone = freeze_standalone(staged);
            let first = fixture.bake(&standalone);
            fixture.write("overlay", &cube([12, 0, 0], [16, 4, 4], "source:changed"));
            (first, fixture.bake(&standalone))
        },
    );
    let model = first.standalone("different_namespace:assembly").unwrap();
    assert_eq!(model.faces.len(), 12);
    close(
        face(model, Direction::North).positions,
        [[0.5, 0.5, 0.], [0.5, 0., 0.], [0., 0., 0.], [0., 0.5, 0.]],
    );
    let overlay = model.faces[6..]
        .iter()
        .find(|f| f.quad.facing == Direction::North)
        .unwrap();
    close(
        overlay.quad.positions,
        [
            [1., 1., 0.5],
            [1., 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [0.5, 1., 0.5],
        ],
    );
    assert_eq!(
        model
            .faces
            .iter()
            .filter(|f| f.quad.material.sprite == "source:base")
            .count(),
        6
    );
    assert_eq!(
        model
            .faces
            .iter()
            .filter(|f| f.quad.material.sprite == "source:overlay")
            .count(),
        6
    );
    assert!(first.standalone("source:base").is_none());
    assert!(first.standalone("source:overlay").is_none());
    let model = second.standalone("different_namespace:assembly").unwrap();
    assert_eq!(model.faces.len(), 12);
    let overlay = model.faces[6..]
        .iter()
        .find(|f| f.quad.facing == Direction::North)
        .unwrap();
    close(
        overlay.quad.positions,
        [
            [1., 0.25, 0.],
            [1., 0., 0.],
            [0.75, 0., 0.],
            [0.75, 0.25, 0.],
        ],
    );
    assert_eq!(overlay.quad.material.sprite, "source:changed");
}

#[test]
fn standalone_file_and_baker_keep_a_child_of_a_rejected_model() {
    let fixture = Fixture::new();
    fixture.write("rejected", r#"{"loader":"source:unregistered"}"#);
    fixture.write(
        "child",
        r##"{"parent":"source:rejected","textures":{"side":"source:child"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#side"}}}]}"##,
    );
    let models = with_test_registrations(
        |event| {
            event.register_model("source:child");
            event.register("result:child", ["source:child"], |bake| {
                bake.append_dependency("source:child", [0; 3], false)
            });
        },
        |staged| fixture.bake(&freeze_standalone(staged)),
    );
    for id in ["source:child", "result:child"] {
        let model = models.standalone(id).unwrap();
        assert_eq!(model.faces.len(), 1);
        assert_eq!(face(model, Direction::Up).material.sprite, "source:child");
    }
}

#[test]
fn standalone_missing_dependency_drops_entire_model_and_logs_registering_mod() {
    let fixture = Fixture::new();
    fixture.write("base", &cube([0, 0, 0], [8, 8, 8], "source:base"));
    let (models, logs) = crate::test_log::capture(|| {
        with_test_registrations(
            |event| {
                combined(event, &["source:base", "source:missing"]);
                rotated(event, "third_namespace:missing_rotated", [0, 1, 0], true);
            },
            |staged| fixture.bake(&freeze_standalone(staged)),
        )
    });
    assert!(models.standalone("different_namespace:assembly").is_none());
    assert!(
        models
            .standalone("third_namespace:missing_rotated")
            .is_none()
    );
    assert!(
        models.baked.is_empty(),
        "no partially baked dependency is published"
    );
    for id in [
        "different_namespace:assembly",
        "third_namespace:missing_rotated",
    ] {
        assert!(
            logs.iter()
                .any(|line| line.contains("mod registering_mod standalone model")
                    && line.contains(id)),
            "{logs:?}"
        );
    }
}

#[test]
fn standalone_failed_append_does_not_latch_and_propagated_error_discards_only_its_assembly() {
    let fixture = Fixture::new();
    fixture.write("base", &cube([0, 0, 0], [8, 8, 8], "source:base"));
    fixture.write("other", &cube([8, 8, 8], [16, 16, 16], "source:other"));
    let (models, logs) = crate::test_log::capture(|| {
        with_test_registrations(
            |event| {
                event.register("result:handled", ["source:base"], |bake| {
                    bake.append_dependency("source:base", [0; 3], false)?;
                    let err = bake.append_dependency("source:other", [0; 3], false);
                    assert_eq!(
                        err,
                        Err(StandaloneModelBakeError::UndeclaredDependency(
                            "source:other".into()
                        ))
                    );
                    // A failed append leaves the assembly intact and does not poison later appends.
                    bake.append_dependency("source:base", [0, 1, 0], false)?;
                    Ok(())
                });
                event.register("result:propagated", ["source:base"], |bake| {
                    bake.append_dependency("source:base", [0; 3], false)?;
                    bake.append_dependency("source:other", [0; 3], false)
                });
                event.register("result:valid", ["source:other"], |bake| {
                    bake.append_dependency("source:other", [0; 3], false)
                });
            },
            |staged| fixture.bake(&freeze_standalone(staged)),
        )
    });
    let handled = models.standalone("result:handled").unwrap();
    assert_eq!(handled.faces.len(), 12);
    assert!(
        handled
            .faces
            .iter()
            .all(|face| face.quad.material.sprite == "source:base")
    );
    assert!(models.standalone_slot("result:propagated").is_none());
    let valid = models.standalone("result:valid").unwrap();
    assert_eq!(valid.faces.len(), 6);
    assert_eq!(
        models.model_count(),
        2,
        "the failed baker must not consume a slot"
    );
    assert!(
        logs.iter().any(|line| line
            .contains("mod registering_mod standalone model result:propagated")
            && line.contains("undeclared dependency source:other")),
        "{logs:?}"
    );
    assert!(
        !logs.iter().any(|line| line.contains("result:handled")),
        "{logs:?}"
    );
}

#[test]
fn standalone_non_panic_failure_retries_on_next_reload() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let fixture = Fixture::new();
    fixture.write("base", &cube([0, 0, 0], [8, 8, 8], "source:base"));
    let attempts = Arc::new(AtomicUsize::new(0));
    let calls = attempts.clone();
    let (first, second) = with_test_registrations(
        move |event| {
            let calls = calls.clone();
            event.register("result:recoverable", ["source:base"], move |bake| {
                if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                    return Err(StandaloneModelBakeError::Rejected("not ready".into()));
                }
                bake.append_dependency("source:base", [0; 3], false)
            });
            event.register("result:late_file", ["source:late"], |bake| {
                bake.append_dependency("source:late", [0; 3], false)
            });
        },
        |staged| {
            let standalone = freeze_standalone(staged);
            let first = fixture.bake(&standalone);
            fixture.write("late", &cube([0, 0, 0], [16, 16, 16], "source:late"));
            let second = fixture.bake(&standalone);
            (first, second)
        },
    );
    assert!(first.standalone("result:recoverable").is_none());
    assert!(first.standalone("result:late_file").is_none());
    assert_eq!(
        second.standalone("result:recoverable").unwrap().faces.len(),
        6
    );
    assert_eq!(
        second.standalone("result:late_file").unwrap().faces.len(),
        6
    );
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
}

#[test]
fn standalone_panicking_baker_is_not_retried_and_later_baker_can_publish() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let fixture = Fixture::new();
    fixture.write("base", &cube([0, 0, 0], [8, 8, 8], "source:base"));
    let attempts = Arc::new(AtomicUsize::new(0));
    let panics = attempts.clone();
    with_test_registrations(
        move |event| {
            let panics = panics.clone();
            event.register("result:slot", std::iter::empty::<&str>(), move |_| {
                panics.fetch_add(1, Ordering::Relaxed);
                panic!("test baker panic");
            });
            event.register("result:slot", ["source:base"], |bake| {
                bake.append_dependency("source:base", [0; 3], false)
            });
        },
        |staged| {
            let standalone = freeze_standalone(staged);
            for _ in 0..2 {
                let models = fixture.bake(&standalone);
                assert_eq!(models.standalone("result:slot").unwrap().faces.len(), 6);
                assert_eq!(models.model_count(), 1);
            }
        },
    );
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
}
