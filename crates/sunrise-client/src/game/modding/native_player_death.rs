//! A server player's last death location for native mods, through a
//! `PlayerRef` (docs/modding/src/how-to/native-mods.md): `Player.getLastDeathLocation` and
//! `setLastDeathLocation`, as a `GlobalPos` (the dimension's key and the
//! block), so a death in a dimension that is not open still reads. It is kept
//! in the player's record (swapped into `level.last_death` for the player's
//! turn), saved as `LastDeathLocation`, and sent with their next respawn
//! packet, which is what the recovery compass follows. Players resolve
//! through the shared player-handle path (`native_server.rs`), so reads return
//! None for a stale handle, for a player who isn't online here or whose record
//! is swapped out, and when the player never died. Writes reach that same
//! resident record, even when the player is dead.
use super::Game;
use mod_api::{GlobalPos, Identifier, PlayerRef};
use sunrise_core::position::BlockPos;

impl Game<'_> {
    /// The dimension and block of the player's last death
    /// (`Player.getLastDeathLocation`).
    pub(super) fn player_last_death_location(&self, player: PlayerRef) -> Option<GlobalPos> {
        let (dimension, [x, y, z]) = self.native_player_last_death_record(player)?;
        let dimension = Identifier::parse(&dimension).ok()?;
        Some(GlobalPos::new(dimension, BlockPos::new(x, y, z)))
    }

    /// `Player.setLastDeathLocation`: stores where the player died, or
    /// forgets it for None. The client hears of it with the player's next
    /// respawn, as in vanilla. False if the server player's state is unavailable.
    pub(super) fn player_set_last_death_location(
        &mut self,
        player: PlayerRef,
        location: Option<GlobalPos>,
    ) -> bool {
        self.native_as_resident_player(player, |game| {
            game.level.last_death = location.map(|l| {
                let p = l.pos();
                (l.dimension().to_string(), [p.x, p.y, p.z])
            });
        })
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use mod_api::__private::game::*;

    use super::super::Game;
    use mod_api::{GlobalPos, Identifier, PlayerRef};
    use sunrise_core::position::BlockPos;

    /// A block of the overworld.
    fn at([x, y, z]: [i32; 3]) -> GlobalPos {
        GlobalPos::new(
            Identifier::parse("minecraft:overworld").unwrap(),
            BlockPos::new(x, y, z),
        )
    }

    /// A placed guest record.
    pub(in crate::game) fn new_guest(conn: u32, uuid: u128) -> sunrise_client::lan::guest::Guest {
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_client::lan::{Joined, PlayerSender, guest::Guest};
        use sunrise_protocol::packets::configuration::ClientInformation;

        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            20,
            [0.0; 3],
            8,
        );
        guest.placed = true;
        guest
    }

    /// Opens the game to a LAN host with one placed guest (UUID 123, conn 1).
    pub(in crate::game) fn add_guest(game: &mut Game<'_>) -> PlayerRef {
        use sunrise_client::lan::{NetConfig, hub::HubConfig};

        let settings = super::super::lan::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "DeathTest".into(), 2)
        };
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 10,
            simulation_distance: 10,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        };
        game.lan_host(settings, net, config).unwrap();
        let guest = new_guest(1, 123);
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        game.player(123).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn a_players_last_death_location_reads_from_their_record() {
        with_game(|game| {
            let player = add_guest(game);
            // Never died.
            assert_eq!(player.last_death_location(&*game), None);

            // A death records the level and block (`ServerPlayer.die`).
            game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                guest.last_death = Some(("minecraft:overworld".into(), [4, 70, -9]));
            });
            assert_eq!(player.last_death_location(&*game), Some(at([4, 70, -9])));

            // The setter stores it, and None forgets it.
            assert!(player.set_last_death_location(game, Some(at([1, 2, 3]))));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].last_death,
                Some(("minecraft:overworld".into(), [1, 2, 3]))
            );
            assert_eq!(player.last_death_location(&*game), Some(at([1, 2, 3])));
            assert!(player.set_last_death_location(game, None));
            assert_eq!(player.last_death_location(&*game), None);

            // A player who is not online here, and a handle of another
            // player's id, read nothing.
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    player.fields().entity.epoch(),
                    player.fields().entity.id() + 1,
                ),
                player.uuid(),
            );
            {
                let value = Some(("minecraft:overworld".into(), [1, 2, 3]));
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.last_death = value);
            }
            assert_eq!(stale.last_death_location(&*game), None);
            assert!(!stale.set_last_death_location(game, None));
            {
                let value = false;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.placed = value);
            }
            assert_eq!(player.last_death_location(&*game), None);
            {
                let value = true;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.placed = value);
            }

            // Raw saved data crosses the Identifier boundary only here.
            {
                let value = Some(("not a dimension key".into(), [1, 2, 3]));
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.last_death = value);
            }
            assert_eq!(player.last_death_location(&*game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_acting_players_location_reads_and_writes_through_their_turn() {
        with_game(|game| {
            let player = add_guest(game);
            {
                let value = Some(("minecraft:overworld".into(), [4, 70, -9]));
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.last_death = value);
            }
            // During the guest's turn the acting player resolves, and the record is the one swapped into the level.
            let seen = game
                .as_guest(1, |game| {
                    let before = player.last_death_location(&*game);
                    let set = player.set_last_death_location(game, Some(at([1, 2, 3])));
                    (before, set, player.last_death_location(&*game))
                })
                .unwrap();
            assert_eq!(seen, (Some(at([4, 70, -9])), true, Some(at([1, 2, 3]))));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].last_death,
                Some(("minecraft:overworld".into(), [1, 2, 3]))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_death_location_survives_a_playerdata_save_and_load() {
        use sunrise_client::lan::playerdata;

        with_game(|game| {
            let player = add_guest(game);
            assert!(player.set_last_death_location(game, Some(at([4, 70, -9]))));

            let world = std::env::temp_dir().join(format!(
                "sunrise-death-location-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
            ));
            std::fs::create_dir_all(&world).unwrap();
            playerdata::save(
                &game.lan.as_ref().unwrap().hub.guests[0],
                &world,
                &Default::default(),
            );

            // A fresh record of the same player never died until it loads.
            let mut fresh = new_guest(2, 123);
            assert_eq!(fresh.last_death, None);
            assert!(playerdata::load(&mut fresh, &world));
            assert_eq!(
                fresh.last_death,
                Some(("minecraft:overworld".into(), [4, 70, -9]))
            );

            // Put in place of the saved one, it reads through the handle.
            game.lan.as_mut().unwrap().hub.guests.replace(0, fresh);
            assert_eq!(player.last_death_location(&*game), Some(at([4, 70, -9])));
            std::fs::remove_dir_all(&world).unwrap();
        });
    }

    #[test]
    #[ignore = "daily"]
    fn edits_to_the_dead_players_record_follow_the_respawn_copy_policy() {
        use mod_api::{Hand, PlayerDataRefusal};

        // The game fixture publishes engine registries. Run it in its own process.
        const CHILD: &str = "SUNRISE_RESIDENT_PLAYER_SETTERS_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "game::native_player_death::tests::edits_to_the_dead_players_record_follow_the_respawn_copy_policy",
                ])
                .env(CHILD, "1")
                .status()
                .unwrap();
            assert!(result.success());
            return;
        }
        super::super::native_guest_turn_tests::with_guests(|game| {
            let player = game.player(13).unwrap();
            let other = game.player(12).unwrap();
            let pearl = "minecraft:ender_pearl";
            let rock = game
                .item_from_snbt(r#"{id:"minecraft:stone",count:3}"#)
                .unwrap();
            let saved = Some(at([4, 72, -3]));
            for (mode, selected, copied_count) in [(0, 0, None), (3, 5, Some(3))] {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(13, |guest| {
                        guest.health = 0.0;
                        guest.dead = true;
                        guest.game_mode = mode;
                    })
                    .unwrap();
                assert!(game.player_set_item_cooldown(player, pearl, 18));
                let living = game.living(player.entity()).unwrap();
                assert_eq!(living.alive(game), Some(false));
                assert!(
                    living
                        .set_hand_item(game, Hand::Off, Some(rock.clone()))
                        .unwrap()
                );
                game.as_guest(13, |game| {
                    assert!(player.set_selected_slot(game, 5));
                    assert_eq!(
                        game.player_item_cooldowns(player).unwrap()[0].ticks_left,
                        18
                    );
                    assert!(game.player_remove_item_cooldown(player, pearl));
                    assert_eq!(game.player_has_item_cooldown(player, pearl), Some(false));
                    assert_eq!(
                        player.set_inventory_slot(game, 5, Some(rock.clone())),
                        Ok(())
                    );
                    assert_eq!(
                        player.set_ender_chest_slot(game, 8, Some(rock.clone())),
                        Ok(())
                    );
                    assert!(player.set_last_death_location(game, saved.clone()));
                    game.as_guest(12, |game| {
                        assert!(!player.set_last_death_location(game, None));
                        assert_eq!(
                            player.set_inventory_slot(game, 5, None),
                            Err(PlayerDataRefusal::Stale)
                        );
                        assert_eq!(other.last_death_location(game), None);
                    })
                    .unwrap();
                })
                .unwrap();
                let slots = |view: mod_api::InventoryView<'_>| {
                    [5, 40].map(|slot| view.get(slot).map(|stack| stack.count()))
                };
                assert_eq!(player.with_inventory(game, slots), Some([Some(3); 2]));
                assert_eq!(player.respawn(game), Ok(()));
                assert_eq!(player.selected_slot(game), Some(selected));
                assert_eq!(player.with_inventory(game, slots), Some([copied_count; 2]));
                assert_eq!(
                    player.with_ender_chest(game, |view| view.get(8).map(|stack| stack.count())),
                    Some(Some(3))
                );
                assert_eq!(player.last_death_location(game), saved);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_death_records_the_floored_block_and_dimension() {
        with_game(|game| {
            let player = add_guest(game);
            {
                // Dying in the Nether at a fractional position.
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.dimension = "minecraft:the_nether".into();
                    guest.position = [-1.2, 64.9, 7.1];
                    guest.health = 0.0;
                });
            }
            game.guest_die(0, 1);

            let guest = &game.lan.as_ref().unwrap().hub.guests[0];
            assert!(guest.dead);
            assert_eq!(
                guest.last_death,
                Some(("minecraft:the_nether".into(), [-2, 64, 7]))
            );
            // The Nether is not open here; the location still reads.
            assert_eq!(
                player.last_death_location(&*game),
                Some(GlobalPos::new(
                    Identifier::parse("minecraft:the_nether").unwrap(),
                    BlockPos::new(-2, 64, 7)
                ))
            );
            assert!(player.set_last_death_location(game, Some(at([1, 2, 3]))));
            assert_eq!(player.last_death_location(&*game), Some(at([1, 2, 3])));
            game.as_guest(1, |game| {
                assert!(game.player.as_ref().unwrap().health.dead_or_dying());
                assert!(player.set_last_death_location(game, None));
            })
            .unwrap();
            assert_eq!(player.last_death_location(&*game), None);
        });
    }
}
