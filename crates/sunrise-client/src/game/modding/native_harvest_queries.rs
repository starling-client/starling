//! The correct-tool queries a mod can call: `BlockStates::is_correct_tool_for_drops` (a stack
//! alone) and `PlayerRef::has_correct_tool_for_drops` (the answer the game's own digs and
//! breaks use, `Game::acting_can_harvest`).
use super::Game;
use mod_api::{LevelRef, PlayerRef};
use sunrise_world::registry::BlockStateId;

impl Game<'_> {
    /// `Player.hasCorrectToolForDrops(state, level, pos)` and `canHarvestBlock`: the player's
    /// selected slot against `state`, then the mods' `can_harvest_block` and
    /// `player_harvest_check`. A server asks a connected or fake player (their state swapped
    /// in); a client only its local player. None for a stale handle, an unknown state, a dead player,
    /// a level other than the one in place, or a call from inside a harvest hook
    /// (`can_harvest_block`, `player_harvest_check`), which would recurse.
    pub(super) fn player_has_correct_tool_for_drops(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        // From inside a harvest hook the query would run the hook again, without bound.
        if crate::game::hand::HarvestDispatch::running() {
            return None;
        }
        if self.native_level_read_world(level).is_none() || level.dimension() != self.dim.key() {
            return None;
        }
        let mut answer = None;
        if self.is_logical_server() {
            self.native_as_player_handle(player, |game| {
                answer = Some(game.acting_can_harvest(pos, state));
            });
        } else if self.local_player() == Some(player) {
            answer = Some(self.acting_can_harvest(pos, state));
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_block_tables::BlockTables;
    use super::super::native_guest_turn_tests::with_guests;
    use mod_api::__private::game::*;
    use mod_api::{Hooks, HostSide, PlayerHarvestCheckEvent, PlayerRef};
    use std::sync::{Arc, Mutex};
    use sunrise_client::items::ItemStack;

    /// A stack is the correct tool by its own rules only; a player is by their selected slot
    /// (never the offhand), and a block that needs no tool is harvested by an empty hand.
    #[test]
    #[ignore = "daily"]
    fn the_correct_tool_is_the_stacks_rules_and_the_players_selected_slot() {
        with_guests(|game| {
            let state = |name: &str| game.pack.assets.registry.default_state(name).unwrap();
            let (stone, dirt) = (state("minecraft:stone"), state("minecraft:dirt"));
            let pickaxe = ItemStack::new("minecraft:iron_pickaxe", 1);
            let shovel = ItemStack::new("minecraft:iron_shovel", 1);
            // The stack alone: dirt needs no tool, yet a pickaxe is not its correct tool.
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(&pickaxe, stone),
                Some(true)
            );
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(&shovel, stone),
                Some(false)
            );
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(&pickaxe, dirt),
                Some(false)
            );
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(
                    &ItemStack::new("minecraft:stick", 1),
                    stone
                ),
                Some(false)
            );
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(
                    &pickaxe,
                    sunrise_world::registry::BlockStateId(u32::MAX)
                ),
                None
            );

            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            let level = a.level(&*game).unwrap();
            let hold = |game: &mut super::super::Game<'_>,
                        conn,
                        main: Option<ItemStack>,
                        off: Option<ItemStack>| {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |guest| {
                        guest.inventory[guest.selected] = main;
                        guest.offhand = off;
                    })
                    .unwrap();
            };
            hold(game, 12, Some(pickaxe.clone()), None);
            hold(game, 13, None, Some(pickaxe));
            let ask = |game: &mut super::super::Game<'_>, p, s| {
                game.player_has_correct_tool_for_drops(p, level, [0, 64, 0], s)
            };
            assert_eq!(ask(game, a, stone), Some(true));
            assert_eq!(
                ask(game, b, stone),
                Some(false),
                "the offhand does not count"
            );
            assert_eq!(ask(game, a, dirt), Some(true));
            assert_eq!(ask(game, b, dirt), Some(true), "dirt needs no tool");
            // During another player's turn the displaced player still answers from their own slot.
            let (outer, inner) = game
                .as_guest(13, |game| (ask(game, a, stone), ask(game, b, stone)))
                .unwrap();
            assert_eq!((outer, inner), (Some(true), Some(false)));
            // A handle that names nobody here asks nothing.
            let stale = player_ref(
                mod_api::EntityRef::new(a.fields().entity.epoch(), a.fields().entity.id() + 100),
                a.uuid(),
            );
            assert_eq!(ask(game, stale, stone), None);
        });
    }

    /// Asks the player query for a different state from a `player_harvest_check` listener.
    struct Asker(Arc<Mutex<Vec<(PlayerRef, Option<bool>)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Asker {
        fn player_harvest_check(&self, host: HostSide<'_>, _: &mut PlayerHarvestCheckEvent) {
            let HostSide::Server(host) = host else { return };
            let player = host.players().acting_player().unwrap();
            let level = player.level(&*host).unwrap();
            let dirt = host.block_state_id("minecraft:dirt").unwrap();
            let asked = player.has_correct_tool_for_drops(host, level, [0, 64, 0], dirt);
            self.0.lock().unwrap().push((player, asked));
        }
    }

    /// Two fake players answer from their own selected slots and run the ordinary hooks.
    /// From inside those hooks the query refuses even a different state.
    #[test]
    #[ignore = "daily"]
    fn the_query_from_inside_a_harvest_hook_answers_none() {
        let inner = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "asker",
            Box::new(Asker(inner.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let stone = (game as &dyn mod_api::Host)
                    .block_state_id("minecraft:stone")
                    .unwrap();
                game.next_entity = 100;
                let fakes = [
                    (1234, "pickaxe", "minecraft:iron_pickaxe", true),
                    (5678, "shovel", "minecraft:iron_shovel", false),
                ]
                .map(|(uuid, name, item, expected)| {
                    let player = game.fake_player(level, uuid, name).unwrap();
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    let index = hub
                        .guests
                        .identity_index(uuid, player.entity().id())
                        .unwrap();
                    hub.guests.update(index, |guest| {
                        guest.selected = 4;
                        guest.inventory[4] = Some(ItemStack::new(item, 1));
                        guest.offhand = Some(ItemStack::new("minecraft:iron_pickaxe", 1));
                    });
                    (player, expected)
                });
                for (player, expected) in fakes {
                    assert_eq!(
                        player.has_correct_tool_for_drops(game, level, [0, 64, 0], stone),
                        Some(expected)
                    );
                    assert_eq!(
                        std::mem::take(&mut *inner.lock().unwrap()),
                        [(player, None)]
                    );
                }
            });
        });
    }
}
