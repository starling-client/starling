//! Mods' selector tokens (`@allies`, NeoForge's `EntitySelectorManager`): the base selections
//! the command registries parse them with, and their types' filter over the candidates.
use super::Game;
use super::commands::Target;
use mod_api::SelectorSort;
use sunrise_client::command::args::Selector;
use sunrise_client_core::registries::SelectorToken;

/// The base selections of currently enabled registered selector tokens.
pub(super) fn selector_tokens() -> Vec<SelectorToken> {
    let mut tokens = Vec::new();
    for (name, owner, selector) in mod_api::__private::entity_selector_types() {
        if !owner.is_enabled() {
            continue;
        }
        let base = match owner.call("entity_selector", "entity_selector.base", || {
            selector.base()
        }) {
            Ok(base) => base,
            Err(why) => {
                log::warn!("entity selector @{name} base failed: {why}");
                continue;
            }
        };
        tokens.push(SelectorToken {
            name: name.to_owned(),
            tooltip: base.tooltip,
            players_only: base.players_only,
            limit: base.limit.map_or(usize::MAX, |limit| limit as usize),
            sort: match base.sort {
                SelectorSort::Nearest => "nearest",
                SelectorSort::Furthest => "furthest",
                SelectorSort::Random => "random",
                _ => "arbitrary",
            },
        });
    }
    tokens
}

impl Game<'_> {
    /// `select_in_level`, then, for a mod's token, only the candidates its type keeps: after the
    /// base selection and the `[...]` options, before sorting and the limit. A vanilla selector
    /// costs nothing more.
    pub(super) fn native_selector_kept(
        &mut self,
        s: &Selector,
        candidates: Vec<Target>,
        origin: [f64; 3],
    ) -> Vec<(Target, f64)> {
        let selected = self.select_in_level(candidates, s, origin);
        let Some(token) = &s.token else {
            return selected;
        };
        let Some((owner, selector)) = mod_api::__private::entity_selector_type(token) else {
            return Vec::new();
        };
        if !owner.is_enabled() {
            return Vec::new();
        }
        let Ok(source) = self.native_command_origin(self.ui.registries.permission) else {
            return Vec::new();
        };
        let mut kept = Vec::new();
        for (target, distance) in selected {
            if !owner.is_enabled() {
                break;
            }
            let Some(entity) = self.native_command_entity_target(target) else {
                continue;
            };
            let host: &mut dyn mod_api::ServerHost = self;
            match owner.call("entity_selector", "entity_selector.matches", || {
                selector.matches(host, &source, entity)
            }) {
                Ok(true) => kept.push((target, distance)),
                Ok(false) => {}
                Err(why) => {
                    log::warn!("entity selector @{token} failed: {why}");
                    if !owner.is_enabled() {
                        break;
                    }
                }
            }
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::fixture_resident_column;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_core::geometry::Vec3;

    struct Teammates(Arc<Mutex<Vec<(mod_api::CommandSource, u32)>>>);
    impl mod_api::EntitySelectorType for Teammates {
        fn base(&self) -> mod_api::SelectorBase {
            mod_api::SelectorBase::new(false, Some(1), SelectorSort::Nearest, "test.allies")
        }
        fn matches(
            &self,
            _: &mut dyn mod_api::ServerHost,
            source: &mod_api::CommandSource,
            entity: mod_api::EntityRef,
        ) -> bool {
            self.0.lock().unwrap().push((source.clone(), entity.id()));
            matches!(entity.id(), 112 | 115)
        }
    }

    struct PanickingMatches(Arc<Mutex<Vec<u32>>>);
    impl mod_api::EntitySelectorType for PanickingMatches {
        fn base(&self) -> mod_api::SelectorBase {
            mod_api::SelectorBase::new(false, Some(1), SelectorSort::Nearest, "test.broken")
        }
        fn matches(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mod_api::CommandSource,
            entity: mod_api::EntityRef,
        ) -> bool {
            self.0.lock().unwrap().push(entity.id());
            match entity.id() {
                112 | 115 => true,
                113 => panic!("fixture predicate failure"),
                _ => false,
            }
        }
    }

    struct TeamMod {
        allies_calls: Arc<Mutex<Vec<(mod_api::CommandSource, u32)>>>,
        broken_calls: Arc<Mutex<Vec<u32>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for TeamMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_entity_selector("allies", Teammates(self.allies_calls.clone()))
                .unwrap();
            r.register_entity_selector("broken", PanickingMatches(self.broken_calls.clone()))
                .unwrap();
        }
    }

    #[test]
    fn panicking_selector_base_disables_only_that_selector_and_emits_panic_notice() {
        struct PanickingBase(Arc<AtomicUsize>);
        impl mod_api::EntitySelectorType for PanickingBase {
            fn base(&self) -> mod_api::SelectorBase {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("fixture selector base panic")
            }
            fn matches(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mod_api::CommandSource,
                _: mod_api::EntityRef,
            ) -> bool {
                false
            }
        }
        struct HealthyBase;
        impl mod_api::EntitySelectorType for HealthyBase {
            fn base(&self) -> mod_api::SelectorBase {
                mod_api::SelectorBase::new(
                    true,
                    Some(2),
                    SelectorSort::Nearest,
                    "test.healthy_base",
                )
            }
            fn matches(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mod_api::CommandSource,
                _: mod_api::EntityRef,
            ) -> bool {
                true
            }
        }
        struct BasePanicMod(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for BasePanicMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_entity_selector("broken_base", PanickingBase(self.0.clone()))
                    .unwrap();
                r.register_entity_selector("Healthy_1.0", HealthyBase)
                    .unwrap();
            }
        }

        let base_calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "selector_base_panic_fixture",
            Box::new(BasePanicMod(base_calls.clone())),
        )]
        .into();
        let _ = mod_api::testing::rebuild_parts(|| -> Result<(), ()> {
            crate::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                let _ = mod_api::__private::take_hook_panics();
                let first = selector_tokens();
                assert_eq!(first.len(), 1);
                assert_eq!(first[0].name, "Healthy_1.0");
                assert_eq!(base_calls.load(Ordering::Relaxed), 1);

                let notices: Vec<_> = mod_api::__private::take_hook_panics()
                    .into_iter()
                    .filter(|notice| notice.mod_id == "selector_base_panic_fixture")
                    .collect();
                assert_eq!(notices.len(), 1);
                assert_eq!(notices[0].hook, "entity_selector.base");

                // The poisoned selector is skipped on subsequent calls while its sibling stays active.
                let second = selector_tokens();
                assert_eq!(second, first);
                assert_eq!(base_calls.load(Ordering::Relaxed), 1);
                let later_notices: Vec<_> = mod_api::__private::take_hook_panics()
                    .into_iter()
                    .filter(|notice| notice.mod_id == "selector_base_panic_fixture")
                    .collect();
                assert!(later_notices.is_empty());
            });
            Err(())
        });
    }

    #[test]
    #[ignore = "daily"]
    fn synced_selector_tokens_survive_client_registry_refresh() {
        struct ClientMustNotBuildSelector(Arc<AtomicUsize>);
        impl mod_api::EntitySelectorType for ClientMustNotBuildSelector {
            fn base(&self) -> mod_api::SelectorBase {
                self.0.fetch_add(1, Ordering::Relaxed);
                mod_api::SelectorBase::new(
                    true,
                    Some(3),
                    SelectorSort::Nearest,
                    "test.local_allies",
                )
            }

            fn matches(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mod_api::CommandSource,
                _: mod_api::EntityRef,
            ) -> bool {
                true
            }
        }

        struct LocalSelector(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for LocalSelector {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                registrar
                    .register_entity_selector(
                        "local_allies",
                        ClientMustNotBuildSelector(self.0.clone()),
                    )
                    .unwrap();
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "selector_client_fixture",
            Box::new(LocalSelector(calls.clone())),
        )]
        .into();
        let _ = mod_api::testing::rebuild_parts(|| -> Result<(), ()> {
            crate::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                super::super::native_entity_refs::tests::with_client_game(|game| {
                    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(client, "SelectorSyncTest").unwrap();
                    assert!(game.ui.registries.selector_tokens.is_empty());
                    assert_eq!(calls.load(Ordering::Relaxed), 0);
                    let tokens = vec![SelectorToken {
                        name: "remote_allies".into(),
                        tooltip: "test.remote_allies".into(),
                        players_only: true,
                        limit: 1,
                        sort: "nearest",
                    }];
                    game.apply_packet(sunrise_protocol::packets::play::Clientbound::CustomPayload(
                        sunrise_protocol::packets::configuration::CustomPayload {
                            channel: sunrise_client::lan::commands::selector_sync::CHANNEL.into(),
                            data: sunrise_client::lan::commands::selector_sync::encode(&tokens),
                        },
                    ))
                    .unwrap();
                    assert_eq!(game.ui.registries.selector_tokens, tokens);
                    game.build_registries();
                    assert_eq!(game.ui.registries.selector_tokens, tokens);
                    assert_eq!(calls.load(Ordering::Relaxed), 0);
                    for data in [
                        vec![2],
                        vec![9],
                        sunrise_client::lan::commands::selector_sync::encode(&tokens),
                    ] {
                        game.apply_packet(
                            sunrise_protocol::packets::play::Clientbound::CustomPayload(
                                sunrise_protocol::packets::configuration::CustomPayload {
                                    channel: sunrise_client::lan::commands::selector_sync::CHANNEL
                                        .into(),
                                    data,
                                },
                            ),
                        )
                        .unwrap();
                        assert!(game.ui.registries.selector_tokens.is_empty());
                    }
                });
            });
            Err(())
        });
    }

    #[test]
    #[ignore = "daily"]
    fn nearest_teammate_filters_before_limit_and_drops_false_and_panicking_candidates() {
        let allies_calls = Arc::new(Mutex::new(Vec::new()));
        let broken_calls = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "selector_fixture",
            Box::new(TeamMod {
                allies_calls: allies_calls.clone(),
                broken_calls: broken_calls.clone(),
            }),
        )]
        .into();
        let _ = mod_api::testing::rebuild_parts(|| -> Result<(), ()> {
            crate::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                crate::game::native_guest_turn_tests::with_guests(|game| {
                    game.build_registries();
                    // A body and level UUID alone do not identify a server command
                    // source. Enter a real guest turn so its UUID/name/permission
                    // and entity identity are installed by the production path.
                    game.as_guest(12, |game| {
                        fixture_resident_column(&mut game.world, (0, 0));
                        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                        for (id, x) in [(111, 1.0), (112, 2.0), (113, 1.5), (114, 0.5), (115, 3.0)]
                        {
                            game.mobs.push(sunrise_client::mobs::Mob::new(
                                id,
                                kind.clone(),
                                Vec3::new(x, 0.0, 0.0),
                                0.0,
                            ));
                        }
                        let selector = |token: &str| Selector {
                            kind: 'e',
                            include_entities: true,
                            token: Some(token.into()),
                            limit: 1,
                            sort: sunrise_client::command::args::Sort::Nearest,
                            pos: [Some(0.0); 3],
                            ..Default::default()
                        };

                        assert_eq!(game.select(&selector("allies")), [Target::Mob(112)]);
                        {
                            let calls = allies_calls.lock().unwrap();
                            for id in [111, 112, 113, 114, 115] {
                                let (source, _) = calls
                                    .iter()
                                    .find(|(_, candidate)| *candidate == id)
                                    .expect("every mob must be filtered before limit");
                                let mod_api::CommandSource::Player { player, .. } = source else {
                                    panic!("expected the acting player source");
                                };
                                assert_eq!(player.uuid(), 12);
                                assert_eq!(player.entity().id(), 12);
                            }
                        }

                        // A panicking selector disables only itself for the rest of the session,
                        // breaks out before remaining candidates, and emits a panic notice while
                        // its sibling selector keeps working.
                        let _ = mod_api::__private::take_hook_panics();
                        assert_eq!(game.select(&selector("broken")), [Target::Mob(112)]);
                        let calls_after_panic = broken_calls.lock().unwrap().len();
                        assert!(
                            !broken_calls.lock().unwrap().contains(&114)
                                && !broken_calls.lock().unwrap().contains(&115),
                            "poisoned selector must stop without evaluating remaining candidates"
                        );
                        let notices: Vec<_> = mod_api::__private::take_hook_panics()
                            .into_iter()
                            .filter(|notice| notice.mod_id == "selector_fixture")
                            .collect();
                        assert_eq!(notices.len(), 1);
                        assert_eq!(notices[0].hook, "entity_selector.matches");

                        assert!(game.select(&selector("broken")).is_empty());
                        assert_eq!(broken_calls.lock().unwrap().len(), calls_after_panic);
                        assert_eq!(
                            selector_tokens()
                                .iter()
                                .map(|token| token.name.as_str())
                                .collect::<Vec<_>>(),
                            ["allies"]
                        );
                        assert_eq!(game.select(&selector("allies")), [Target::Mob(112)]);
                    })
                    .unwrap();
                });
            });
            Err(())
        });
    }
}
