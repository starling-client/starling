//! Real menu storage and Game journal integration, before hook-route migration.
//! These exercise Menu::click, not the synthetic MenuState planner. They do not
//! assert public hook dispatch.
use super::*;
use crate::game::{
    native_entity_refs::tests::run_fresh_cases, native_guest_turn_tests::with_guests,
};
use sunrise_client::menu::{Input, Kind};

fn fixture(game: &mut Game<'_>) -> (Target, Identity) {
    let mut menu = Box::new(EngineMenu::new(Kind::Inventory));
    menu.carried = Some(ItemStack::new("minecraft:stone", 3));
    *game
        .lan
        .as_mut()
        .unwrap()
        .menus
        .entry(12)
        .or_default()
        .crafting
        .inventory = Some(menu);
    let actor = game.player(12).unwrap().entity();
    (
        Target::admit(game, 12).unwrap(),
        Identity {
            side: Side::Server,
            world: actor.epoch(),
            actor,
            target: actor,
        },
    )
}

fn placement(host: &GameHost<'_, '_>, menu: &EngineMenu) -> EnginePlan {
    let mut preview = menu.clone();
    preview
        .click(&host.game.menu_env(), 9, 0, Input::Pickup)
        .unwrap();
    EnginePlan(Some(preview.snapshot()))
}

fn initial(target: Target, game: &Game<'_>) {
    let (_, menu) = target.read(game).unwrap();
    assert_eq!(menu.carried, Some(ItemStack::new("minecraft:stone", 3)));
    assert_eq!(menu.slot(9), None);
}

#[test]
#[ignore = "daily"]
fn actual_menu_frozen_click_and_outer_abort_use_the_game_journal() {
    if run_fresh_cases(
        "game::transactions::decision::engine::tests::actual_menu_frozen_click_and_outer_abort_use_the_game_journal",
        "SUNRISE_DFA_ENGINE_CHILD",
        &["run"],
    ) {
        return;
    }
    with_guests(|game| {
        let (target, identity) = fixture(game);
        game.storage_scope(|game, _| {
            let mut host = GameHost { game, identity };
            assert_eq!(
                run(&mut host, &target, placement, |_| Reply::Allow),
                Ok(Outcome::Applied)
            );
            let (_, menu) = target.read(host.game).unwrap();
            assert_eq!(menu.carried, None);
            assert_eq!(menu.slot(9), Some(&ItemStack::new("minecraft:stone", 3)));
            ((), false)
        });
        initial(target, game);
        assert!(game.transactions.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn actual_menu_cursor_aba_is_stale_and_undoes_a_nested_commit() {
    if run_fresh_cases(
        "game::transactions::decision::engine::tests::actual_menu_cursor_aba_is_stale_and_undoes_a_nested_commit",
        "SUNRISE_DFA_ENGINE_CHILD",
        &["run"],
    ) {
        return;
    }
    with_guests(|game| {
        let (target, identity) = fixture(game);
        game.storage_scope(|game, _| {
            let mut host = GameHost { game, identity };
            assert_eq!(
                run(&mut host, &target, placement, |host| {
                    host.game.storage_scope(|game, _| {
                        target
                            .edit(game, |menu| {
                                menu.carried = Some(ItemStack::new("minecraft:dirt", 3));
                                menu.carried = Some(ItemStack::new("minecraft:stone", 3));
                            })
                            .unwrap();
                        ((), true)
                    });
                    Reply::Allow
                }),
                Ok(Outcome::Stale(StaleReason::MenuInstance))
            );
            initial(target, host.game);
            assert_eq!(
                run(&mut host, &target, placement, |_| Reply::Allow),
                Ok(Outcome::Applied)
            );
            ((), false)
        });
        initial(target, game);
    });
}

#[test]
#[ignore = "daily"]
fn actual_owner_replacement_is_stale_and_same_owner_reentry_is_busy() {
    if run_fresh_cases(
        "game::transactions::decision::engine::tests::actual_owner_replacement_is_stale_and_same_owner_reentry_is_busy",
        "SUNRISE_DFA_ENGINE_CHILD",
        &["run"],
    ) {
        return;
    }
    with_guests(|game| {
        let (target, identity) = fixture(game);
        game.storage_scope(|game, _| {
            let mut host = GameHost { game, identity };
            assert_eq!(
                run(&mut host, &target, placement, |host| {
                    assert_eq!(
                        run(
                            host,
                            &target,
                            |_, _| panic!("busy planner"),
                            |_| panic!("busy dispatch")
                        ),
                        Err(Admission::Busy)
                    );
                    let same_menu = host
                        .game
                        .lan
                        .as_mut()
                        .unwrap()
                        .menus
                        .get_mut(&12)
                        .unwrap()
                        .crafting
                        .inventory
                        .take();
                    assert!(matches!(Target::admit(host.game, 12), Err(Admission::Busy)));
                    *host
                        .game
                        .lan
                        .as_mut()
                        .unwrap()
                        .menus
                        .get_mut(&12)
                        .unwrap()
                        .crafting
                        .inventory = same_menu;
                    Reply::Allow
                }),
                Ok(Outcome::Stale(StaleReason::MenuInstance))
            );
            initial(target, host.game);
            ((), true)
        });
        assert!(game.transactions.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn actual_owner_nested_edit_and_panic_roll_back_before_the_next_click() {
    if run_fresh_cases(
        "game::transactions::decision::engine::tests::actual_owner_nested_edit_and_panic_roll_back_before_the_next_click",
        "SUNRISE_DFA_ENGINE_CHILD",
        &["run"],
    ) {
        return;
    }
    with_guests(|game| {
        let (target, identity) = fixture(game);
        game.storage_scope(|game, _| {
            let mut host = GameHost { game, identity };
            assert_eq!(
                run(&mut host, &target, placement, |host| {
                    host.game.storage_scope(|game, _| {
                        target
                            .edit(game, |menu| {
                                menu.carried = Some(ItemStack::new("minecraft:dirt", 2))
                            })
                            .unwrap();
                        ((), true)
                    });
                    panic!("recoverable listener failure");
                }),
                Ok(Outcome::ListenerFailed)
            );
            initial(target, host.game);
            assert_eq!(
                run(&mut host, &target, placement, |_| Reply::Allow),
                Ok(Outcome::Applied)
            );
            ((), false)
        });
        initial(target, game);
    });
}
