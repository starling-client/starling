use super::super::*;
use crate::*;
use std::sync::Mutex;

type Trace = Arc<Mutex<Vec<(&'static str, f32)>>>;
#[derive(Clone)]
struct Cadence<const PERIOD: i32> {
    package: &'static str,
    trace: Trace,
}
#[mod_api::sunrise_part]
impl<const PERIOD: i32> MobEffectPart for Cadence<PERIOD> {
    fn should_apply_effect_tick_this_tick(
        &self,
        _: &dyn Host,
        event: &mut ShouldApplyEffectTickThisTickEvent,
    ) {
        assert!(!event.apply);
        event.apply = event.tick_count % PERIOD == 0;
    }
    fn apply_effect_tick(&self, _: &dyn Host, event: &mut ApplyEffectTickEvent<'_>) {
        assert!(event.has_effect(event.effect));
        self.trace
            .lock()
            .unwrap()
            .push((self.package, event.health));
        event.heal(PERIOD as f32);
        if matches!(
            (self.package, event.amplifier),
            ("owner", 99) | ("other", 100)
        ) {
            event.keep = false;
        }
    }
}
#[mod_api::__subscriptions]
impl<const PERIOD: i32> Hooks for Cadence<PERIOD> {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let effect = r.effect_key("owner:comfort");
        r.mob_effect_part(&effect, self.clone()).unwrap();
    }
}

fn package(id: &str, dependency: Option<DependencyKind>) -> PackageInfo {
    PackageInfo::new(
        id.into(),
        "1".into(),
        PackageKind::Code,
        Side::Both,
        vec![id.into()],
        dependency
            .into_iter()
            .map(|kind| {
                PackageDependency::new(
                    "owner".into(),
                    "*".into(),
                    kind,
                    DependencyOrder::After,
                    Side::Both,
                    None,
                    None,
                )
            })
            .collect(),
    )
}

#[test]
fn optional_effect_parts_skip_only_when_the_package_is_absent() {
    for present in [false, true] {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "behaviour",
            Box::new(Cadence::<3> {
                package: "behaviour",
                trace,
            }),
        )]
        .into();
        let mut packages = vec![package("behaviour", Some(DependencyKind::Optional))];
        if present {
            packages.push(package("owner", None));
        }
        crate::load_state::with_test_package_inventory(packages, || {
            crate::testing::with_mods(mods.clone(), || {
                install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
                let ids = if present {
                    vec!["owner:comfort"]
                } else {
                    vec![]
                };
                mod_vocab::__private::resolve_effects(ids, |_| present.then_some(0)).unwrap();
                mod_vocab::__private::validate_effect_part_owners(|_| present.then_some("owner"))
                    .unwrap();
                assert_eq!(
                    mod_vocab::__private::effect_key_by_index(0).is_some(),
                    present
                );
            });
        });
    }
}

#[test]
fn unknown_required_effects_keep_the_key_resolution_diagnostic() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "behaviour",
        Box::new(Cadence::<3> {
            package: "behaviour",
            trace,
        }),
    )]
    .into();
    crate::load_state::with_test_package_inventory(
        vec![package("behaviour", Some(DependencyKind::Required))],
        || {
            crate::testing::with_mods(mods.clone(), || {
                install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
                assert_eq!(
                    mod_vocab::__private::resolve_effects([], |_| None).unwrap_err(),
                    ["mod behaviour requested unknown effect owner:comfort"]
                );
            });
        },
    );
}

#[test]
fn registration_refuses_vanilla_and_duplicate_effect_parts() {
    struct Registration;
    #[mod_api::__subscriptions]
    impl Hooks for Registration {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let part = || Cadence::<2> {
                package: "owner",
                trace: Arc::new(Mutex::new(Vec::new())),
            };
            let vanilla = r.effect_key("minecraft:poison");
            assert_eq!(
                r.mob_effect_part(&vanilla, part()),
                Err(MobEffectPartError::VanillaEffect { effect: vanilla })
            );
            let custom = r.effect_key("owner:comfort");
            r.mob_effect_part(&custom, part()).unwrap();
            assert_eq!(
                r.mob_effect_part(&custom, part()),
                Err(MobEffectPartError::Duplicate {
                    effect: custom,
                    part: std::any::type_name::<Cadence<2>>().into(),
                })
            );
        }
    }
    let errors = install_parts(
        &[ModEntry::new("owner", Box::new(Registration))],
        &crate::parts::tests::rule_schema(),
        |_| false,
    )
    .unwrap_err();
    assert!(errors.iter().any(|error| error.contains("owner:comfort")));
}

#[test]
fn registration_refuses_malformed_effect_key_distinctly_from_duplicate() {
    struct Registration;
    #[mod_api::__subscriptions]
    impl Hooks for Registration {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let part = Cadence::<2> {
                package: "owner",
                trace: Arc::new(Mutex::new(Vec::new())),
            };
            let invalid = r.effect_key("Bad Id");
            assert_eq!(
                r.mob_effect_part(&invalid, part),
                Err(MobEffectPartError::InvalidKey { effect: invalid })
            );
        }
    }
    let errors = install_parts(
        &[ModEntry::new("owner", Box::new(Registration))],
        &crate::parts::tests::rule_schema(),
        |_| false,
    )
    .unwrap_err();
    assert_eq!(
        errors,
        [
            "mod owner: invalid registry key \"minecraft:Bad Id\"",
            &format!(
                "mod owner attached {} to an invalid effect key",
                std::any::type_name::<Cadence<2>>()
            ),
        ]
    );
}

struct World;
impl Host for World {}
crate::impl_host_areas!(World);

fn install(mods: &[ModEntry]) -> EffectKey {
    install_parts(mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mod_vocab::__private::resolve_effects(["owner:comfort"], |_| Some(0)).unwrap();
    mod_vocab::__private::effect_key_by_index(0).unwrap()
}

#[test]
fn effect_owner_runs_first_even_when_the_dependent_dispatches_first() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![
        // The package resolver can put dependents first (order = "before").
        ModEntry::new(
            "other",
            Box::new(Cadence::<6> {
                package: "other",
                trace: trace.clone(),
            }),
        ),
        ModEntry::new(
            "behaviour",
            Box::new(Cadence::<3> {
                package: "behaviour",
                trace: trace.clone(),
            }),
        ),
        ModEntry::new(
            "owner",
            Box::new(Cadence::<2> {
                package: "owner",
                trace: trace.clone(),
            }),
        ),
    ]
    .into();
    crate::load_state::with_test_package_inventory(
        vec![
            package("owner", None),
            package("behaviour", Some(DependencyKind::Required)),
            package("other", Some(DependencyKind::Required)),
        ],
        || {
            crate::testing::with_mods(mods.clone(), || {
                let effect = install(&mods);
                let active_effects = [effect];
                mod_vocab::__private::validate_effect_part_owners(|_| Some("owner")).unwrap();
                for (ticks, amplifier, expected, expected_trace, expected_keep) in [
                    (1, 0, vec![], vec![], true),
                    (
                        2,
                        0,
                        vec![EffectTickChange::Heal(2.0)],
                        vec![("owner", 10.0)],
                        true,
                    ),
                    (
                        3,
                        0,
                        vec![EffectTickChange::Heal(3.0)],
                        vec![("behaviour", 10.0)],
                        true,
                    ),
                    (
                        6,
                        0,
                        vec![
                            EffectTickChange::Heal(2.0),
                            EffectTickChange::Heal(6.0),
                            EffectTickChange::Heal(3.0),
                        ],
                        vec![("owner", 10.0), ("other", 10.0), ("behaviour", 10.0)],
                        true,
                    ),
                    (
                        6,
                        99,
                        vec![EffectTickChange::Heal(2.0)],
                        vec![("owner", 10.0)],
                        false,
                    ),
                    (
                        6,
                        100,
                        vec![EffectTickChange::Heal(2.0), EffectTickChange::Heal(6.0)],
                        vec![("owner", 10.0), ("other", 10.0)],
                        false,
                    ),
                ] {
                    trace.lock().unwrap().clear();
                    let mut changes = Vec::new();
                    let mut keep = true;
                    let prepared = std::cell::Cell::new(false);
                    mod_vocab::__private::dispatch_mob_effect_tick(
                        &World,
                        ticks,
                        effect,
                        amplifier,
                        |effect, amplifier| {
                            prepared.set(true);
                            mod_vocab::__private::ApplyEffectTickCall {
                                effect,
                                dimension: "minecraft:overworld",
                                entity: Some(7),
                                amplifier,
                                holder: LivingRef::new(EntityRef::new(WorldEpoch::new(1), 7)),
                                health: 10.0,
                                max_health: 20.0,
                                food: None,
                                active_effects: &active_effects,
                                changes: &mut changes,
                                keep: &mut keep,
                            }
                        },
                    );
                    assert_eq!(prepared.get(), !expected_trace.is_empty());
                    assert_eq!(changes, expected);
                    assert_eq!(*trace.lock().unwrap(), expected_trace);
                    assert_eq!(keep, expected_keep);
                }
            });
        },
    );
}

#[test]
fn a_removed_dependency_row_cannot_authorize_a_foreign_effect_part() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "behaviour",
        Box::new(Cadence::<3> {
            package: "behaviour",
            trace,
        }),
    )]
    .into();
    crate::load_state::with_test_package_inventory(
        vec![
            package("owner", None),
            package("behaviour", Some(DependencyKind::Required)),
        ],
        || {
            // The manifest still declares owner; the effective graph lost that row.
            crate::load_state::with_test_dependencies(Default::default(), || {
                crate::testing::with_mods(mods.clone(), || {
                    install(&mods);
                    let errors =
                        mod_vocab::__private::validate_effect_part_owners(|_| Some("owner"))
                            .unwrap_err();
                    assert_eq!(
                        errors,
                        [format!(
                            "mod behaviour attached {} to effect owner:comfort, declared by package owner; declare owner as a dependency",
                            std::any::type_name::<Cadence<3>>()
                        )]
                    );
                });
            });
        },
    );
}
