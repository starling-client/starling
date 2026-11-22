//! The render groups of mods' particles (`register_particle_groups`). The engine keeps each group's
//! live particles apart from the built-in ones, as `ParticleEngine.particles` keeps one
//! `ParticleGroup` per `ParticleRenderType`, and resolves each particle type's group when its
//! sprite sets load, so a spawn, a tick and a draw never look a group up by name.
use super::Particle;
use mod_api::{
    __private::game::{CallbackOwner, ParticleGroupRegistration},
    Answered, Identifier, ParticleGroup,
};
use std::{collections::BTreeMap, sync::OnceLock};

pub(super) struct Factory {
    registration: ParticleGroupRegistration,
    owner: CallbackOwner,
}

/// Accepted factories in registration order, frozen before the first particle engine is built.
static REGISTERED: OnceLock<Vec<Factory>> = OnceLock::new();

/// Freeze the mods' particle group factories at startup. Duplicate IDs fail registration
/// without invoking any factory or committing the table.
pub fn register_particle_groups(
    groups: Vec<Answered<ParticleGroupRegistration>>,
) -> Result<(), String> {
    let factories = freeze(groups)?;
    if REGISTERED.set(factories).is_err() {
        log::warn!("Particle groups already registered; keeping the first table");
    }
    Ok(())
}

pub(super) fn registered() -> &'static [Factory] {
    REGISTERED.get().map_or(&[], Vec::as_slice)
}

fn freeze(groups: Vec<Answered<ParticleGroupRegistration>>) -> Result<Vec<Factory>, String> {
    let mut ids = BTreeMap::new();
    let mut factories = Vec::new();
    for entry in groups {
        let registration = entry.answer;
        let id = &registration.id;
        let mod_id = entry.mod_id;
        if id.namespace().is_empty() || id.path().is_empty() {
            log::warn!("Mod {mod_id}: particle group {id} must have a nonempty namespace and path");
            continue;
        }
        if let Some(first_mod) = ids.insert(id.clone(), mod_id) {
            return Err(format!(
                "Duplicate particle group registration: {id} (registered by {first_mod} and {mod_id})"
            ));
        }
        let Some(owner) = CallbackOwner::new(mod_id) else {
            log::warn!("Particle group {id} has no installed owner {mod_id}");
            continue;
        };
        factories.push(Factory {
            registration,
            owner,
        });
    }
    Ok(factories)
}

/// A group's engine-owned definition and live particles. Vanilla's `ParticleGroup` keeps
/// its own queue and limit the same way.
pub(super) struct Group {
    pub(super) id: Identifier,
    pub(super) definition: ParticleGroup,
    pub(super) particles: Vec<Particle>,
}

impl Group {
    /// Invoke each accepted factory once, in registration order. Only constructed groups
    /// occupy slots, so the group's index is its dense handle in this engine.
    pub(super) fn all(factories: &[Factory]) -> Vec<Self> {
        factories
            .iter()
            .filter_map(
                |Factory {
                     registration,
                     owner,
                 }| {
                    let id = &registration.id;
                    match owner.call(
                        "particle group factory",
                        "construct_particle_group",
                        &registration.factory,
                    ) {
                        Ok(definition) => Some(Self {
                            id: id.clone(),
                            definition,
                            particles: Vec::new(),
                        }),
                        Err(error) => {
                            log::warn!("Particle group {id}: {error}");
                            None
                        }
                    }
                },
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{native::tests::Scene, *};
    use super::{Group, ParticleGroup, freeze};
    use mod_api::{
        __private::game::RegistrationContextNew, Identifier, ParticleBatch, ParticleLevelView,
        ParticleProvider, RegistrationContext,
    };
    use std::{collections::BTreeMap, sync::Mutex};
    use sunrise_assets::resource_source::Resource;

    fn group(id: &str, blend: ParticleBlend, particles: &[&str]) -> Group {
        Group {
            id: Identifier::parse(id).unwrap(),
            definition: ParticleGroup {
                blend,
                particles: particles.iter().map(|&p| p.into()).collect(),
            },
            particles: Vec::new(),
        }
    }

    #[test]
    fn factories_run_once_per_engine_in_order_and_never_on_sprite_reload() {
        struct Register(Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Register {
            fn register_particle_groups(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterParticleGroupsEvent<'_>,
            ) {
                for (id, call, member) in [
                    (
                        Identifier::parse("mymod:glow").unwrap(),
                        "glow",
                        "mymod:glow",
                    ),
                    (
                        Identifier::parse("minecraft:glow").unwrap(),
                        "minecraft_glow",
                        "mymod:plain",
                    ),
                    (
                        Identifier::from_parts("", "empty_ns").unwrap(),
                        "empty namespace",
                        "mymod:orbit",
                    ),
                    (
                        Identifier::parse("mymod:").unwrap(),
                        "empty path",
                        "mymod:orbit",
                    ),
                    (
                        Identifier::parse("mymod:shade").unwrap(),
                        "shade",
                        "mymod:shade",
                    ),
                ] {
                    let calls = Arc::clone(&self.0);
                    event.register(id, move || {
                        calls.lock().unwrap().push(call);
                        ParticleGroup {
                            blend: ParticleBlend::Opaque,
                            particles: vec![member.into()],
                        }
                    });
                }
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "mymod",
            Box::new(Register(calls.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            let mut staged = Vec::new();
            mod_api::dispatch_register_particle_groups(&RegistrationContext::new(), &mut staged);
            let factories = freeze(staged).unwrap();
            assert!(calls.lock().unwrap().is_empty());

            let pack = Pack::new();
            let mut engine = ParticleEngine::with_group_factories(
                &pack.root,
                &pack.atlas,
                &HashSet::new(),
                &factories,
            );
            let handles = |engine: &ParticleEngine| {
                ["mymod:glow", "mymod:shade", "mymod:orbit", "mymod:plain"]
                    .map(|name| engine.mod_sets[name].group)
            };
            assert_eq!(handles(&engine), [Some(0), Some(2), None, Some(1)]);
            assert_eq!(*calls.lock().unwrap(), ["glow", "minecraft_glow", "shade"]);

            engine.reload(
                &ResourceSource::indexed(Default::default()),
                &pack.atlas,
                &HashSet::new(),
            );
            assert!(engine.mod_sets.is_empty());
            pack.reload(&mut engine);
            assert_eq!(handles(&engine), [Some(0), Some(2), None, Some(1)]);
            assert_eq!(*calls.lock().unwrap(), ["glow", "minecraft_glow", "shade"]);

            let next = ParticleEngine::with_group_factories(
                &pack.root,
                &pack.atlas,
                &HashSet::new(),
                &factories,
            );
            assert_eq!(handles(&next), [Some(0), Some(2), None, Some(1)]);
            assert_eq!(
                *calls.lock().unwrap(),
                [
                    "glow",
                    "minecraft_glow",
                    "shade",
                    "glow",
                    "minecraft_glow",
                    "shade"
                ]
            );
        });
    }

    #[test]
    fn duplicate_particle_group_id_fails_without_calling_factories() {
        struct RegisterOne(&'static str, Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for RegisterOne {
            fn register_particle_groups(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterParticleGroupsEvent<'_>,
            ) {
                let id = self.0;
                let calls = Arc::clone(&self.1);
                event.register(Identifier::parse(id).unwrap(), move || {
                    calls.lock().unwrap().push(id);
                    ParticleGroup {
                        blend: ParticleBlend::Opaque,
                        particles: Vec::new(),
                    }
                });
            }
        }

        struct RegisterTwice(&'static str, Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for RegisterTwice {
            fn register_particle_groups(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::RegisterParticleGroupsEvent<'_>,
            ) {
                for _ in 0..2 {
                    let id = self.0;
                    let calls = Arc::clone(&self.1);
                    event.register(Identifier::parse(id).unwrap(), move || {
                        calls.lock().unwrap().push(id);
                        ParticleGroup {
                            blend: ParticleBlend::Opaque,
                            particles: Vec::new(),
                        }
                    });
                }
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            mod_api::ModEntry::new("mod_a", Box::new(RegisterOne("mymod:glow", calls.clone()))),
            mod_api::ModEntry::new("mod_b", Box::new(RegisterOne("mymod:glow", calls.clone()))),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            let mut staged = Vec::new();
            mod_api::dispatch_register_particle_groups(&RegistrationContext::new(), &mut staged);
            let error = freeze(staged).err().expect("duplicate ID must fail");
            assert!(error.contains("mymod:glow"), "{error}");
            assert!(
                error.contains("mod_a") && error.contains("mod_b"),
                "{error}"
            );
            assert!(calls.lock().unwrap().is_empty());
        });

        let mods = vec![mod_api::ModEntry::new(
            "mymod",
            Box::new(RegisterTwice("minecraft:dup", calls.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            let mut staged = Vec::new();
            mod_api::dispatch_register_particle_groups(&RegistrationContext::new(), &mut staged);
            let error = freeze(staged).err().expect("duplicate ID must fail");
            assert!(error.contains("minecraft:dup"), "{error}");
            assert!(calls.lock().unwrap().is_empty());
        });
    }

    /// Moves a particle to a fixed spot on each tick, so a drawn quad says where it is.
    struct Stay;
    impl ParticleProvider for Stay {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            for particle in particles {
                particle.position = particle.previous;
            }
        }
    }

    /// A pack with the vanilla `flame` and mod sets `plain`, `glow`, `shade` and `orbit`, each with
    /// the one sprite `<ns>:particle/one`, and an atlas that holds every sprite.
    struct Pack {
        root: ResourceSource,
        atlas: HashMap<String, Rect>,
    }
    impl Pack {
        fn new() -> Self {
            let mut files = BTreeMap::new();
            let mut json = |ns: &str, path: &str, textures: &str| {
                files.insert(
                    format!("assets/{ns}/particles/{path}"),
                    Resource::bytes(textures.as_bytes().to_vec()),
                );
            };
            json(
                "minecraft",
                "flame.json",
                r#"{"textures":["minecraft:flame"]}"#,
            );
            for set in ["plain", "glow", "shade", "orbit"] {
                json(
                    "mymod",
                    &format!("{set}.json"),
                    r#"{"textures":["mymod:one"],"sunrise:behaviour":{"lifetime":2,"size":[0.5,0.5]}}"#,
                );
            }
            let rect = Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 16,
                h: 16,
            };
            let atlas = HashMap::from([
                ("particle/flame".to_owned(), rect),
                ("mymod:particle/one".to_owned(), rect),
            ]);
            Self {
                root: ResourceSource::indexed(files),
                atlas,
            }
        }
        /// An engine whose groups are supplied as already constructed definitions.
        fn engine(&self, groups: Vec<Group>) -> ParticleEngine {
            let mut e =
                ParticleEngine::with_group_factories(&self.root, &self.atlas, &HashSet::new(), &[]);
            e.groups = groups;
            e.resolve_groups();
            e
        }
        fn reload(&self, e: &mut ParticleEngine) {
            e.reload(&self.root, &self.atlas, &HashSet::new());
        }
    }

    fn spawn(e: &mut ParticleEngine, world: &World, options: &ParticleOptions, x: f64) {
        let at = [x, 0.0, 0.0];
        let mut p = match &options.typed_particle {
            Some((value, _)) => e
                .make_typed_particle(world, value, at, [0.0; 3], 1)
                .unwrap(),
            None => e.make(options, at, [0.0; 3], 1),
        }
        .unwrap();
        p.lifetime = 2;
        p.size = 0.5;
        p.velocity = [0.0; 3];
        e.push(p);
    }
    /// The x of the centre of each quad `render` draws, in draw order, and its layer.
    fn drawn(e: &ParticleEngine) -> Vec<(i32, bool)> {
        let mut vertices = Vec::new();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.0,
            &|_| 0,
            &mut vertices,
        );
        vertices
            .chunks(4)
            .map(|quad| {
                let x = quad.iter().map(|v| v.pos[0]).sum::<f32>() / 4.0;
                let translucent = quad[0].layer & crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY != 0;
                (x.round() as i32, translucent)
            })
            .collect()
    }

    #[test]
    fn a_registered_groups_particles_draw_after_the_built_in_ones_and_expire_normally() {
        let (scene, pack) = (Scene::new(), Pack::new());
        let level = scene.level();
        let mut e = pack.engine(vec![
            group(
                "mymod:glow",
                ParticleBlend::Translucent,
                &["mymod:glow", "mymod:orbit"],
            ),
            group(
                "mymod:shade",
                ParticleBlend::Opaque,
                &["mymod:shade", "mymod:glow"],
            ),
        ]);
        // Each type resolved once at load: the first group to list a type keeps it.
        let handle = |name: &str| e.mod_sets[name].group;
        assert_eq!(
            [
                handle("mymod:glow"),
                handle("mymod:orbit"),
                handle("mymod:shade"),
                handle("mymod:plain")
            ],
            [Some(0), Some(0), Some(1), None]
        );
        // A provider's particle is blended on its own, and an orbit type's declaration is opaque:
        // each takes its group's blend.
        let [shade] = super::super::native::tests::unit_types(["shade"]);
        e.set_block_state_source(|| Some(Arc::new(Vec::<mod_api::BlockState>::new())));
        e.providers.insert(
            shade.key(),
            mod_api::__private::bind_particle_provider(shade.key(), Arc::new(Stay)),
        );
        let typed = ParticleOptions::typed(shade.options(()).typed().unwrap().clone()).unwrap();
        pack.reload(&mut e);
        // Spawned in the reverse of the order the groups draw in.
        spawn(&mut e, level.world, &typed, 3.0);
        for (name, x) in [("mymod:glow", 2.0), ("mymod:orbit", 4.0)] {
            spawn(&mut e, level.world, &ParticleOptions::named(name), x);
        }
        spawn(
            &mut e,
            level.world,
            &ParticleOptions::named("mymod:plain"),
            1.0,
        );
        spawn(
            &mut e,
            level.world,
            &ParticleOptions::simple(ParticleType::Flame),
            0.0,
        );
        e.tick(&level);
        assert_eq!(
            (
                e.particles.len(),
                e.groups[0].particles.len(),
                e.groups[1].particles.len()
            ),
            (2, 2, 1)
        );
        assert_eq!(e.len(), 5);
        // The built-in group first, as before (the plain mod particle and the flame, opaque); then
        // group 0, translucent; then group 1, opaque, though its provider's particle is blended.
        assert_eq!(
            drawn(&e),
            [(1, false), (0, false), (2, true), (4, true), (3, false)]
        );
        // Lifetime 2: gone after the same ticks, from every group, and nothing is left drawn.
        e.tick(&level);
        e.tick(&level);
        assert_eq!(e.len(), 5);
        e.tick(&level);
        assert_eq!((e.len(), e.group_count()), (0, 0));
        assert!(e.is_empty() && drawn(&e).is_empty());
    }

    #[test]
    fn each_group_has_its_own_limit_and_a_reload_drops_only_provider_particles_of_a_group() {
        let (scene, pack) = (Scene::new(), Pack::new());
        let level = scene.level();
        let mut e = pack.engine(vec![
            group("mymod:glow", ParticleBlend::Translucent, &["mymod:glow"]),
            group("mymod:shade", ParticleBlend::Opaque, &["mymod:shade"]),
        ]);
        let [shade] = super::super::native::tests::unit_types(["shade"]);
        e.set_block_state_source(|| Some(Arc::new(Vec::<mod_api::BlockState>::new())));
        e.providers.insert(
            shade.key(),
            mod_api::__private::bind_particle_provider(shade.key(), Arc::new(Stay)),
        );
        let typed = ParticleOptions::typed(shade.options(()).typed().unwrap().clone()).unwrap();
        pack.reload(&mut e);
        let options = ParticleOptions::named("mymod:glow");
        let mut full = e.make(&options, [0.0; 3], [0.0; 3], 1).unwrap();
        full.lifetime = 100;
        e.groups[0].particles = vec![full; MAX_PARTICLES];
        spawn(&mut e, level.world, &typed, 0.0);
        for name in ["mymod:glow", "mymod:plain"] {
            spawn(&mut e, level.world, &ParticleOptions::named(name), 0.0);
        }
        e.tick(&level);
        // Group 0 is full, so its newcomer is refused; the other groups and the built-in one
        // still admit theirs.
        let sizes = (
            e.groups[0].particles.len(),
            e.groups[1].particles.len(),
            e.particles.len(),
        );
        assert_eq!(sizes, (MAX_PARTICLES, 1, 1));
        // A reload drops the particles a controller moves (the provider's sprite-set handles
        // belong to the load) in a group as in the built-in one; the declarative particles stay and
        // still draw in their group.
        pack.reload(&mut e);
        assert_eq!(
            (
                e.groups[0].particles.len(),
                e.groups[1].particles.len(),
                e.particles.len()
            ),
            (MAX_PARTICLES, 0, 1)
        );
        assert_eq!(e.typed_sets[&shade.key()].group, Some(1));
    }
}
