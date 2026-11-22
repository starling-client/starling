//! Engine registration and vanilla resource conversion for model layers.
use super::Part;
use mod_api::{
    __private::game::{CallbackOwner, LayerDefinitionRegistration},
    Answered, LayerDefinition, ModelPart, ModelPolygon, ModelVertex, PartPose,
};
use std::{collections::HashMap, path::Path, sync::OnceLock};

/// A vanilla layer's part tree from `entity-models.json`.
pub fn from_part(part: &Part) -> ModelPart {
    let p = part.pose;
    let mut pose = PartPose::offset_and_rotation(p[0], p[1], p[2], p[3], p[4], p[5]);
    pose.x_scale = p[6];
    pose.y_scale = p[7];
    pose.z_scale = p[8];
    let polygons = part
        .polygons
        .iter()
        .map(|poly| {
            ModelPolygon::new(
                poly.vertices
                    .map(|v| ModelVertex::new([v[0], v[1], v[2]], v[3], v[4])),
                poly.normal,
            )
        })
        .collect();
    let children = part
        .children
        .iter()
        .map(|c| (c.name.clone(), from_part(c)))
        .collect();
    ModelPart::new(polygons, children, pose)
}

static REGISTERED: OnceLock<HashMap<String, LayerDefinition>> = OnceLock::new();

/// Validate and deduplicate mod layer registrations (keeping the last supplier per ID, matching
/// NeoForge's `ClientHooks.registerLayerDefinition`), fail on exact collisions with vanilla's
/// layer keys in `entity-models.json` (matching `LayerDefinitions.createRoots`'s
/// `ImmutableMap.Builder.build()`), invoke each winning supplier once, and publish the realized
/// layers once at client startup.
pub fn register(
    layers: Vec<Answered<LayerDefinitionRegistration>>,
    vanilla_root: &Path,
) -> Result<(), String> {
    register_into(&REGISTERED, layers, vanilla_root)
}

fn load_vanilla_models(vanilla_root: &Path) -> Result<super::EntityModelDocument, String> {
    let path = vanilla_root.join("entity-models.json");
    let doc = super::EntityModelDocument::load(vanilla_root).map_err(|error| {
        format!(
            "failed to load vanilla layer definitions from {}: {error}",
            path.display()
        )
    })?;
    if !doc.value().is_object() {
        return Err(format!(
            "invalid vanilla layer definitions in {}: expected a top-level JSON object",
            path.display()
        ));
    }
    Ok(doc)
}

fn register_into(
    slot: &OnceLock<HashMap<String, LayerDefinition>>,
    layers: Vec<Answered<LayerDefinitionRegistration>>,
    vanilla_root: &Path,
) -> Result<(), String> {
    if slot.get().is_some() {
        log::warn!("model layers were registered already; these are left out");
        return Ok(());
    }
    let mut by_id: HashMap<&str, usize> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for (index, entry) in layers.iter().enumerate() {
        let id = entry.answer.id.as_str();
        let mod_id = entry.mod_id;
        let valid = id
            .split_once('#')
            .and_then(|(location, _)| location.split_once(':'))
            .is_some_and(|(namespace, path)| {
                mod_api::Identifier::from_parts(namespace, path).is_ok()
            });
        if !valid {
            log::warn!("Mod {mod_id}: model layer {id:?} is not <namespace>:<path>#<layer>");
            continue;
        }
        if by_id.insert(id, index).is_none() {
            order.push(id);
        }
    }
    let definitions = if order.is_empty() {
        HashMap::new()
    } else {
        let vanilla = load_vanilla_models(vanilla_root)?;
        for &id in &order {
            if vanilla.value().get(id).is_some() {
                let mod_id = layers[by_id[id]].mod_id;
                return Err(format!(
                    "Mod {mod_id}: model layer {id} collides with a vanilla layer definition"
                ));
            }
        }
        let mut definitions = HashMap::new();
        for id in order {
            let entry = &layers[by_id[id]];
            let LayerDefinitionRegistration { id, supplier, .. } = &entry.answer;
            let mod_id = entry.mod_id;
            let Some(owner) = CallbackOwner::new(mod_id) else {
                log::warn!("Model layer {id} has no installed owner {mod_id}");
                continue;
            };
            match owner.call("model layer supplier", "build_layer_definition", supplier) {
                Ok(definition) => {
                    definitions.insert(id.clone(), definition);
                }
                Err(error) => log::warn!("Model layer {id}: {error}"),
            }
        }
        definitions
    };
    if slot.set(definitions).is_err() {
        log::warn!("model layers were registered already; these are left out");
    }
    Ok(())
}

/// The realized definition of the layer a native mod registered as `id`.
pub fn registered(id: &str) -> Option<&'static LayerDefinition> {
    REGISTERED.get()?.get(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{CubeDeformation, CubeListBuilder, MeshDefinition};

    #[test]
    fn suppliers_only_run_for_the_last_valid_ids_at_freeze() {
        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::{Hooks, ModEntry, RegisterLayerDefinitionsEvent, RegistrationContext};
        use std::sync::{Arc, Mutex};

        struct Layers(&'static [(&'static str, i32)], Arc<Mutex<Vec<i32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Layers {
            fn register_layer_definitions(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterLayerDefinitionsEvent<'_>,
            ) {
                for &(id, offset) in self.0 {
                    let calls = self.1.clone();
                    event.register(id, move || {
                        calls.lock().unwrap().push(offset);
                        let mut mesh = MeshDefinition::new();
                        mesh.add_or_replace_child(
                            mesh.root(),
                            "head",
                            CubeListBuilder::create(),
                            PartPose::offset(0.0, offset as f32, 0.0),
                        );
                        LayerDefinition::create(mesh, 64, 32)
                    });
                }
            }
        }

        let vanilla_root =
            std::env::temp_dir().join(format!("model-layers-freeze-{}", std::process::id()));
        std::fs::create_dir_all(&vanilla_root).unwrap();
        std::fs::write(
            vanilla_root.join("entity-models.json"),
            r#"{"minecraft:zombie#main": {}}"#,
        )
        .unwrap();

        let calls = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "first",
                    Box::new(Layers(
                        &[
                            ("minecraft:golem#main", 1),
                            ("first:golem#main", 6),
                            ("first:golem#armor:outer#extra", 4),
                            ("first:golem#", 13),
                            ("first:#main", 14),
                            (":golem#main", 15),
                            ("first:golem", 21),
                            ("golem#main", 22),
                            ("First:golem#main", 23),
                        ],
                        calls.clone(),
                    )),
                ),
                ModEntry::new(
                    "second",
                    Box::new(Layers(
                        &[("first:golem#main", 9), ("second:golem#main", 12)],
                        calls.clone(),
                    )),
                ),
            ]
            .into(),
            || {
                let mut suppliers = Vec::new();
                mod_api::dispatch_register_layer_definitions(
                    &RegistrationContext::new(),
                    &mut suppliers,
                );
                assert!(calls.lock().unwrap().is_empty());
                register(suppliers, &vanilla_root).unwrap();
                assert_eq!(*calls.lock().unwrap(), [1, 9, 4, 13, 14, 15, 12]);

                for (id, expected) in [
                    ("minecraft:golem#main", 1.0),
                    ("first:golem#armor:outer#extra", 4.0),
                    ("first:golem#", 13.0),
                    ("first:#main", 14.0),
                    (":golem#main", 15.0),
                ] {
                    let model = registered(id).unwrap().bake_root();
                    assert_eq!(model.get_child("head").unwrap().y, expected, "{id}");
                }
                for _ in 0..2 {
                    let model = registered("first:golem#main").unwrap().bake_root();
                    assert_eq!(model.get_child("head").unwrap().y, 9.0);
                }
                let second = registered("second:golem#main").unwrap().bake_root();
                assert_eq!(second.get_child("head").unwrap().y, 12.0);

                let mut late = Vec::new();
                mod_api::dispatch_register_layer_definitions(
                    &RegistrationContext::new(),
                    &mut late,
                );
                register(late, &vanilla_root).unwrap();
                assert_eq!(*calls.lock().unwrap(), [1, 9, 4, 13, 14, 15, 12]);
            },
        );
        let _ = std::fs::remove_dir_all(&vanilla_root);
    }

    #[test]
    fn vanilla_layer_collision_and_invalid_document_fail_without_calling_suppliers() {
        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::{Hooks, ModEntry, RegisterLayerDefinitionsEvent, RegistrationContext};
        use std::sync::{Arc, Mutex};

        struct Layers(&'static [(&'static str, i32)], Arc<Mutex<Vec<i32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Layers {
            fn register_layer_definitions(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterLayerDefinitionsEvent<'_>,
            ) {
                for &(id, offset) in self.0 {
                    let calls = self.1.clone();
                    event.register(id, move || {
                        calls.lock().unwrap().push(offset);
                        LayerDefinition::create(MeshDefinition::new(), 64, 32)
                    });
                }
            }
        }

        let base = std::env::temp_dir().join(format!("model-layers-errors-{}", std::process::id()));
        let missing = base.join("missing");
        let malformed = base.join("malformed");
        let wrong_shape = base.join("wrong-shape");
        let valid = base.join("valid");
        for dir in [&missing, &malformed, &wrong_shape, &valid] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(malformed.join("entity-models.json"), "{not json").unwrap();
        std::fs::write(
            wrong_shape.join("entity-models.json"),
            r#"["not-an-object"]"#,
        )
        .unwrap();
        std::fs::write(
            valid.join("entity-models.json"),
            r#"{"minecraft:zombie#main": {}}"#,
        )
        .unwrap();

        let calls = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "first",
                Box::new(Layers(
                    &[("first:golem#main", 1), ("minecraft:zombie#main", 2)],
                    calls.clone(),
                )),
            )]
            .into(),
            || {
                let collect = || {
                    let mut suppliers = Vec::new();
                    mod_api::dispatch_register_layer_definitions(
                        &RegistrationContext::new(),
                        &mut suppliers,
                    );
                    suppliers
                };

                for (dir, expected) in [
                    (&missing, "entity-models.json"),
                    (&malformed, "entity-models.json"),
                    (&wrong_shape, "top-level JSON object"),
                ] {
                    let slot = OnceLock::new();
                    let error = register_into(&slot, collect(), dir).unwrap_err();
                    assert!(error.contains(expected), "{error}");
                    assert!(slot.get().is_none());
                    assert!(calls.lock().unwrap().is_empty());
                }

                let slot = OnceLock::new();
                let error = register_into(&slot, collect(), &valid).unwrap_err();
                assert!(error.contains("minecraft:zombie#main"), "{error}");
                assert!(error.contains("first"), "{error}");
                assert!(slot.get().is_none());
                assert!(calls.lock().unwrap().is_empty());
            },
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// `HumanoidModel.createMesh(CubeDeformation.NONE, 0)` built with these
    /// builders bakes what vanilla baked for the zombie
    /// (`minecraft:zombie#main` in the prepared `entity-models.json`):
    /// `cargo test -p sunrise-client-lib --lib model_layers -- --ignored`
    /// with SUNRISE_TEST_RESOURCES set to the prepared resources.
    #[test]
    #[ignore = "daily"]
    fn the_humanoid_mesh_bakes_as_vanillas_zombie() {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("SUNRISE_TEST_RESOURCES"),
        );
        let vanilla =
            super::super::load_model(&root, "minecraft:zombie#main").expect("zombie#main");
        let vanilla = from_part(&vanilla);

        // `HumanoidModel.createMesh`: (part, parent, texOffs, mirror, origin, size, grow, pose).
        let half = CubeDeformation::new(0.5);
        let none = CubeDeformation::NONE;
        let (z4, limb) = ([-4.0, -8.0, -4.0], [4.0, 12.0, 4.0]);
        #[rustfmt::skip]
        let parts = [
            ("head", "", (0, 0), false, z4, [8.0; 3], none, [0.0; 3]),
            ("hat", "head", (32, 0), false, z4, [8.0; 3], half, [0.0; 3]),
            ("body", "", (16, 16), false, [-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], none, [0.0; 3]),
            ("right_arm", "", (40, 16), false, [-3.0, -2.0, -2.0], limb, none, [-5.0, 2.0, 0.0]),
            ("left_arm", "", (40, 16), true, [-1.0, -2.0, -2.0], limb, none, [5.0, 2.0, 0.0]),
            ("right_leg", "", (0, 16), false, [-2.0, 0.0, -2.0], limb, none, [-1.9, 12.0, 0.0]),
            ("left_leg", "", (0, 16), true, [-2.0, 0.0, -2.0], limb, none, [1.9, 12.0, 0.0]),
        ];
        let mut mesh = MeshDefinition::new();
        for (name, parent, (u, v), mirror, origin, size, grow, [x, y, z]) in parts {
            let parent = mesh.child(mesh.root(), parent).unwrap_or(mesh.root());
            let cubes = CubeListBuilder::create()
                .tex_offs(u, v)
                .set_mirror(mirror)
                .add_box_with(origin, size, grow);
            mesh.add_or_replace_child(parent, name, cubes, PartPose::offset(x, y, z));
        }
        let ours = LayerDefinition::create(mesh, 64, 64).bake_root();

        fn same(path: &str, ours: &ModelPart, vanilla: &ModelPart) {
            assert_eq!(ours.store_pose(), vanilla.store_pose(), "{path} pose");
            assert_eq!(ours.polygons, vanilla.polygons, "{path} faces");
            let mut names: Vec<_> = ours.children().map(|(n, _)| n).collect();
            let mut theirs: Vec<_> = vanilla.children().map(|(n, _)| n).collect();
            names.sort_unstable();
            theirs.sort_unstable();
            assert_eq!(names, theirs, "{path} children");
            for (name, child) in ours.children() {
                same(
                    &format!("{path}/{name}"),
                    child,
                    vanilla.get_child(name).unwrap(),
                );
            }
        }
        same("root", &ours, &vanilla);
    }
}
