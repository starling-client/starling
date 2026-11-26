//! #2704: a crash save never writes the half-done turn of a player a panic interrupted.

use crate::game::{execution_role::ExecutionRole, native_guest_turn_tests::with_guests};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Records who each `player_logged_out` listener call is for.
struct Logouts(std::sync::Arc<Mutex<Vec<u128>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Logouts {
    fn player_logged_out(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::PlayerLoggedOutEvent<'_>,
    ) {
        self.0.lock().unwrap().push(event.player);
    }
}

/// A mod whose logout listener enters a new turn for guest 13 via `set_game_mode`.
/// Its inner callback panics in `player_game_mode_changed`, but the dispatch boundary
/// catches it before the turn unwinds.
struct PanicsInShutdown;
#[mod_api::__subscriptions]
impl mod_api::Hooks for PanicsInShutdown {
    fn player_logged_out(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::PlayerLoggedOutEvent<'_>,
    ) {
        if event.player == 13 {
            host.player(13)
                .expect("guest 13 was online")
                .set_game_mode(host, mod_api::GameMode::Creative);
        }
    }
    fn player_game_mode_changed(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::PlayerGameModeChangedEvent<'_>,
    ) -> mod_api::Flow {
        panic!("mod panic in a new turn");
    }
}

fn player_file(root: &Path, uuid: u128) -> PathBuf {
    sunrise_client::lan::playerdata::path(root, uuid)
}

fn advancements_file(root: &Path, uuid: u128) -> PathBuf {
    root.join("players").join("advancements").join(format!(
        "{}.json",
        sunrise_protocol::crypto::uuid_string(uuid)
    ))
}

/// A half-done turn: the engine panics after the player's inventory changes.
fn taking_turn(game: &mut crate::game::Game<'_>) {
    game.inventory[0] = Some(sunrise_client::items::ItemStack {
        item: "minecraft:diamond".into(),
        count: 3,
        damage: 0,
        extra: None,
    });
    panic!("engine panic in a turn");
}

enum Interruption {
    Single,
    Nested,
    AfterFirstCheckpoint,
    ModePanic,
}

/// Run the real turn helpers and save path, including a turn interrupted after
/// shutdown has already decided which players to skip once, and a contained
/// callback panic during logout that is not an interrupted turn.
fn scenario(interruption: Interruption) {
    with_guests(|game| {
        game.options.execution_role = ExecutionRole::DedicatedServer;
        let root = tempfile::tempdir().unwrap();
        let root = root.path().to_path_buf();
        game.world_root = root.clone();
        game.save = std::sync::Arc::new(sunrise_client::save::SaveDir::new(
            root.clone(),
            sunrise_client::save::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        game.level.name = "interrupted turn".into();
        game.session_lock = Some(sunrise_client::save::SessionLock::acquire(&root).unwrap());
        game.world_adopted = true;
        assert!(
            !game.advancement_tree().list.is_empty(),
            "the advancement tree is loaded, so a save would write advancement files"
        );
        for conn in [12, 13] {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                })
                .unwrap();
            for file in [
                player_file(&root, conn.into()),
                advancements_file(&root, conn.into()),
            ] {
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, format!("last saved of {conn}")).unwrap();
            }
        }
        let read = |uuid: u128| {
            (
                std::fs::read(player_file(&root, uuid)).unwrap(),
                std::fs::read(advancements_file(&root, uuid)).unwrap(),
            )
        };
        let (before_12, before_13) = (read(12), read(13));
        // Guest 12 rides a boat alone; a save would move it into the player file.
        let boat = 700;
        game.boats.push(crate::game::boats::Boat::new(
            boat,
            "oak".into(),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        ));
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| guest.vehicle = Some(boat))
            .unwrap();

        let panic = catch_unwind(AssertUnwindSafe(|| {
            if matches!(interruption, Interruption::Nested) {
                game.as_guest(12, |outer| outer.as_guest(13, taking_turn));
            } else {
                game.as_guest(12, taking_turn);
            }
        }))
        .expect_err("the panic still propagates out of the turn helpers");
        assert_eq!(
            crate::game::dedicated::panic_message(panic.as_ref()),
            "engine panic in a turn"
        );
        assert!(game.lan_guest_swaps.is_empty() && game.lan_acting.is_none());

        let initially_interrupted: &[u32] = match interruption {
            Interruption::Nested => &[12, 13],
            _ => &[12],
        };
        assert_eq!(game.interrupted_turns, initially_interrupted);

        let mode_panic = matches!(interruption, Interruption::ModePanic);
        if matches!(interruption, Interruption::AfterFirstCheckpoint) {
            // Reproduce the state left by an engine error in a new turn after the first
            // shutdown checkpoint. The final save must refresh the earlier exclusion.
            game.exclude_interrupted_players(false, |_| {});
            assert!(game.crash_save.excludes(12));
            assert!(!game.crash_save.excludes(13));
            let panic = catch_unwind(AssertUnwindSafe(|| game.as_guest(13, taking_turn)))
                .expect_err("the later turn must also unwind");
            let message = crate::game::dedicated::panic_message(panic.as_ref());
            assert_eq!(message, "engine panic in a turn");
            assert!(game.lan_guest_swaps.is_empty() && game.lan_acting.is_none());
            assert_eq!(game.interrupted_turns, [12, 13]);
            game.finish_server_save(true, false).unwrap();
        } else {
            let logouts = std::sync::Arc::new(Mutex::new(Vec::new()));
            let entry = if mode_panic {
                mod_api::ModEntry::new("panics", Box::new(PanicsInShutdown))
            } else {
                mod_api::ModEntry::new("logouts", Box::new(Logouts(logouts.clone())))
            };
            mod_platform::testing::with_mods(vec![entry].into(), || {
                game.run_server_lifecycle(true, |_| Ok(super::ServerRunOutcome::Orderly))
                    .unwrap();
                let panics = mod_api::__private::take_hook_panics();
                if mode_panic {
                    assert_eq!(
                        panics,
                        [mod_api::__private::HookPanicReport {
                            mod_id: "panics",
                            hook: "player_game_mode_changed",
                        }]
                    );
                } else {
                    assert!(panics.is_empty());
                }
            });
            if !mode_panic {
                // An interrupted player stays online: each login still gets its one logout.
                let mut left = logouts.lock().unwrap();
                left.sort_unstable();
                assert_eq!(*left, [12, 13]);
            }
        }

        assert_eq!(
            read(12),
            before_12,
            "the interrupted player keeps the last saved files"
        );
        assert!(
            game.boats.iter().any(|b| b.id == boat),
            "the interrupted player's boat stays in the world, not in a file nobody writes"
        );
        if matches!(
            interruption,
            Interruption::Nested | Interruption::AfterFirstCheckpoint
        ) {
            assert_eq!(
                read(13),
                before_13,
                "so does the player interrupted by the nested or later turn"
            );
        } else {
            assert_ne!(
                read(13).0,
                before_13.0,
                "a player whose turn was not interrupted is saved as ever"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_crash_save_keeps_the_files_of_players_whose_turn_a_panic_interrupted() {
    scenario(Interruption::Single);
    scenario(Interruption::Nested);
    scenario(Interruption::AfterFirstCheckpoint);
    scenario(Interruption::ModePanic);
}
