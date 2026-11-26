//! A player's food and experience for native server mods, through a
//! `PlayerRef` (docs/modding/src/how-to/native-mods.md): the `FoodData` and `Player` reads the
//! handle's `food()` doesn't give, and direct writes. Reads return None for a
//! stale handle or a player who isn't online here, as `food()` does. Writes
//! return false for those, for a dead player, and, inside another player's
//! turn, for anyone but that player. What changes reaches the player's client as any
//! change to their food or experience does.
use super::Game;
use mod_api::{PlayerExperience, PlayerRef};
use sunrise_client::entities::Experience;

impl Game<'_> {
    /// Whether `player` still names the same player on this server.
    fn native_player_valid(&self, player: PlayerRef) -> bool {
        self.player_handle_read(player).is_some()
    }
    /// Runs `f` with `player`'s state in place, when the handle is current
    /// and they are alive (`native_as_player`).
    fn native_player_write(&mut self, player: PlayerRef, f: impl FnOnce(&mut Self)) -> bool {
        self.native_player_valid(player) && self.native_as_player_handle(player, f)
    }
    /// The acting player's food data, inside `native_player_write`.
    fn native_food_mut(&mut self) -> Option<&mut sunrise_world::food::FoodData> {
        self.player.as_mut().map(|p| &mut p.health.food)
    }

    /// `FoodData.needsFood`: the food level is below 20.
    pub(super) fn player_needs_food(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_food(player)?.level < 20)
    }
    /// `FoodData.hasEnoughFood`: the food level is above 6, enough to
    /// sprint.
    pub(super) fn player_has_enough_food(&self, player: PlayerRef) -> Option<bool> {
        Some(self.player_food(player)?.level as f32 > 6.0)
    }
    /// `Player.canEat(always)`: an invulnerable player (creative), `always`
    /// (food that can always be eaten) or a food level below 20.
    pub(super) fn player_can_eat(&self, player: PlayerRef, always: bool) -> Option<bool> {
        let invulnerable = self.player_abilities(player)?.invulnerable;
        Some(invulnerable || always || self.player_needs_food(player)?)
    }
    /// `FoodData.eat(nutrition, saturationModifier)`: adds `nutrition` to
    /// the food level (0 to 20) and `nutrition * modifier * 2` to the
    /// saturation, which stays at most the food level.
    pub(super) fn player_feed(
        &mut self,
        player: PlayerRef,
        nutrition: i32,
        saturation_modifier: f32,
    ) -> bool {
        self.native_player_write(player, |game| {
            if let Some(food) = game.native_food_mut() {
                food.eat(nutrition, saturation_modifier);
            }
        })
    }
    /// `Player.causeFoodExhaustion`: adds exhaustion (at most 40), as
    /// sprinting or jumping does. An invulnerable player (creative) or a
    /// spectator gets none, and the call still returns true.
    pub(super) fn player_add_exhaustion(&mut self, player: PlayerRef, amount: f32) -> bool {
        self.native_player_write(player, |game| {
            if let Some(p) = game.player.as_mut() {
                p.cause_food_exhaustion(amount);
            }
        })
    }
    /// `FoodData.setFoodLevel`: sets the food level as it is, unclamped,
    /// as vanilla does.
    pub(super) fn player_set_food_level(&mut self, player: PlayerRef, level: i32) -> bool {
        self.native_player_write(player, |game| {
            if let Some(food) = game.native_food_mut() {
                food.food = level;
            }
        })
    }
    /// `FoodData.setSaturation`: sets the saturation as it is, unclamped.
    pub(super) fn player_set_saturation(&mut self, player: PlayerRef, saturation: f32) -> bool {
        self.native_player_write(player, |game| {
            if let Some(food) = game.native_food_mut() {
                food.saturation = saturation;
            }
        })
    }
    /// Sets the exhaustion as it is (Paper's `HumanEntity#setExhaustion`;
    /// vanilla's `FoodData` only adds to it).
    pub(super) fn player_set_exhaustion(&mut self, player: PlayerRef, exhaustion: f32) -> bool {
        self.native_player_write(player, |game| {
            if let Some(food) = game.native_food_mut() {
                food.exhaustion = exhaustion;
            }
        })
    }

    /// `Player.experienceLevel`, `experienceProgress` and `totalExperience`.
    pub(super) fn player_experience(&self, player: PlayerRef) -> Option<PlayerExperience> {
        Some(match self.player_read_both(player)? {
            super::native_server::PlayerRead::Guest(g) => {
                PlayerExperience::from(mod_api::__private::built::PlayerExperience {
                    level: g.xp_level,
                    progress: g.xp_progress,
                    total: g.xp_total,
                })
            }
            super::native_server::PlayerRead::Acting { .. } => {
                PlayerExperience::from(mod_api::__private::built::PlayerExperience {
                    level: self.experience.level,
                    progress: self.experience.progress,
                    total: self.experience.total,
                })
            }
        })
    }
    /// `Player.getXpNeededForNextLevel`: the points the current level needs
    /// to fill.
    pub(super) fn player_xp_needed_for_next_level(&self, player: PlayerRef) -> Option<i32> {
        Some(Experience::needed(self.player_experience(player)?.level))
    }
    /// `ServerPlayer.setExperiencePoints`: sets the progress toward the next
    /// level to `points` of the points it needs, kept below a full bar. The
    /// level and the total are kept.
    pub(super) fn player_set_experience_points(&mut self, player: PlayerRef, points: i32) -> bool {
        self.native_player_write(player, |game| {
            let limit = Experience::needed(game.experience.level) as f32;
            let max = (limit - 1.0) / limit;
            // Mth.clamp delegates to Math.min for the upper bound, preserving NaN.
            let progress = points as f32 / limit;
            game.experience.progress = if progress < 0.0 {
                0.0
            } else if progress.is_nan() {
                progress
            } else {
                progress.min(max)
            };
        })
    }
    /// `ServerPlayer.setExperienceLevels`: sets the level, keeping the
    /// progress and the total.
    pub(super) fn player_set_experience_levels(&mut self, player: PlayerRef, levels: i32) -> bool {
        self.native_player_write(player, |game| game.set_experience_levels(levels))
    }
    /// Writes the signed `Player.totalExperience` field, keeping the level and progress.
    pub(super) fn player_set_total_experience(&mut self, player: PlayerRef, total: i32) -> bool {
        self.native_player_write(player, |game| game.experience.total = total)
    }
    /// `ServerPlayer.giveExperiencePoints`: adds or takes signed points. Mending is part of
    /// `ExperienceOrb.playerTouch`, not this write.
    pub(super) fn player_give_experience_points(&mut self, player: PlayerRef, points: i32) -> bool {
        self.native_player_write(player, |game| game.give_experience_points(points))
    }
    /// `ExperienceOrb.repairPlayerItems` for `player` (#5593): the orb pickup's Mending, the
    /// points left over.
    pub(super) fn player_repair_items_with_xp(
        &mut self,
        player: PlayerRef,
        points: i32,
    ) -> Option<i32> {
        let mut rest = None;
        self.native_player_write(player, |game| rest = Some(game.mending(points)));
        rest
    }
    /// `Player.giveExperienceLevels`: adds `levels`, or takes them off when
    /// negative, as the anvil and the enchanting table charge with. Below 0
    /// the level, the total and the progress toward the next level are all 0.
    pub(super) fn player_give_levels(&mut self, player: PlayerRef, levels: i32) -> bool {
        self.native_player_write(player, |game| game.give_levels(levels))
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use mod_api::{EntityRef, WorldEpoch};
    use sunrise_client::{items::ItemStack, motion::PlayerMotion};
    use sunrise_core::geometry::Vec3;

    /// The acting player with a body, as a hook's turn finds them.
    fn acting_player(game: &mut super::Game<'_>) -> PlayerRef {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
        game.player(123).unwrap()
    }

    /// A mod spends experience on Mending as an orb's pickup does: 2 durability a point, the
    /// rest returned and the player's experience untouched (#5593).
    #[test]
    #[ignore = "daily"]
    fn repairing_with_experience_returns_what_mending_leaves() {
        with_game(|game| {
            let player = acting_player(game);
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.damage = 1000;
            sword.enchant("minecraft:mending", 1);
            game.inventory[game.selected] = Some(sword);
            assert_eq!(player.repair_items_with_xp(game, 7), Some(0));
            let damage =
                |game: &super::Game<'_>| game.inventory[game.selected].as_ref().unwrap().damage;
            assert_eq!(damage(game), 986);
            // ExperienceOrb.repairPlayerItems: 4 damage costs 2 of 7 points, and the 5 left find
            // nothing more to repair.
            game.inventory[game.selected].as_mut().unwrap().damage = 4;
            assert_eq!(player.repair_items_with_xp(game, 7), Some(5));
            assert_eq!(damage(game), 0);
            assert_eq!(player.repair_items_with_xp(game, 7), Some(7));
            let xp = player.experience(game).unwrap();
            assert_eq!((xp.level, xp.total), (0, 0));
            let stale = mod_api::__private::game::player_ref(
                EntityRef::new(WorldEpoch::new(u64::MAX), 7),
                player.uuid(),
            );
            assert_eq!(stale.repair_items_with_xp(game, 7), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn signed_points_bypass_mending() {
        with_game(|game| {
            let player = acting_player(game);
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.damage = 1000;
            sword.enchant("minecraft:mending", 1);
            game.inventory[game.selected] = Some(sword);
            assert!(player.give_experience_points(game, 7));
            let xp = player.experience(game).unwrap();
            assert_eq!((xp.level, xp.progress, xp.total), (1, 0.0, 7));
            assert!(player.give_experience_points(game, -7));
            let xp = player.experience(game).unwrap();
            assert_eq!((xp.level, xp.progress, xp.total), (0, 0.0, 0));
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().damage, 1000);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn levels_are_added_taken_and_clamped_at_zero() {
        with_game(|game| {
            let player = acting_player(game);
            game.experience.progress = 0.25;
            assert!(game.player_give_levels(player, 3));
            assert_eq!(game.experience.level, 3);
            assert_eq!(
                game.experience.progress, 0.25,
                "a level add keeps the progress"
            );
            game.experience.total = 40;
            assert!(game.player_give_levels(player, -5));
            assert_eq!(game.experience.level, 0, "below zero clamps to zero");
            assert_eq!(game.experience.total, 0, "the clamp clears the total");
            assert_eq!(
                game.experience.progress, 0.0,
                "the clamp clears the progress"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_signed_total_is_set_alone() {
        with_game(|game| {
            let player = acting_player(game);
            game.experience.level = 5;
            game.experience.progress = 0.5;
            assert!(game.player_set_total_experience(player, 1234));
            assert_eq!(game.player_experience(player).unwrap().total, 1234);
            assert_eq!(game.experience.level, 5, "the level is kept");
            assert_eq!(game.experience.progress, 0.5, "the progress is kept");
            assert!(game.player_set_total_experience(player, i32::MIN));
            let xp = player.experience(game).unwrap();
            assert_eq!((xp.level, xp.progress, xp.total), (5, 0.5, i32::MIN));
            assert_eq!(game.player_score, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_score_uses_the_signed_hook_adjusted_grant() {
        struct TakePoint;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for TakePoint {
            fn player_xp_change(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::PlayerXpChangeEvent<'_>,
            ) -> mod_api::Flow {
                event.amount = -1;
                mod_api::Flow::Continue
            }
        }
        let mods = vec![mod_api::ModEntry::new("take-point", Box::new(TakePoint))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let player = acting_player(game);
                game.lan_acting_name = Some("ScoreProbe".into());
                assert!(player.give_experience_points(game, 1));
                assert_eq!(game.player_score, -1);
                assert_eq!(game.experience.total, 0);

                game.player_score = i32::MIN;
                assert!(player.give_experience_points(game, 1));
                assert_eq!(game.player_score, i32::MAX);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn enchantment_debits_ask_but_absolute_level_writes_only_report() {
        use std::sync::{Arc, Mutex};
        struct Observe(Arc<Mutex<Vec<(&'static str, i32, i32)>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Observe {
            fn player_xp_level_change(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::PlayerXpLevelChangeEvent<'_>,
            ) -> mod_api::Flow {
                let level = host
                    .players()
                    .acting_player()
                    .unwrap()
                    .experience(host)
                    .unwrap()
                    .level;
                self.0.lock().unwrap().push(("before", level, event.levels));
                if event.levels == -4 {
                    mod_api::Flow::Cancel
                } else {
                    mod_api::Flow::Continue
                }
            }

            fn player_level_change(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::PlayerLevelChangeEvent<'_>,
            ) {
                let level = host
                    .players()
                    .acting_player()
                    .unwrap()
                    .experience(host)
                    .unwrap()
                    .level;
                assert_eq!(event.to, level);
                self.0.lock().unwrap().push(("after", event.from, level));
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "xp",
            Box::new(Observe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let player = acting_player(game);
                game.lan_acting_name = Some("LevelProbe".into());
                game.lan_acting_permission = Some(0);
                // A player's turn on the server thread: `acting_player` names the
                // body `bind_local_player` gave the turn.
                game.lan_acting = Some(1);
                assert!(player.set_experience_levels(game, 5));
                assert!(player.set_experience_levels(game, 5));
                game.enchantment_performed(4);
                assert_eq!(game.experience.level, 5);
                game.enchantment_performed(2);
                assert_eq!(game.experience.level, 3);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("after", 0, 5),
                        ("before", 5, -4),
                        ("before", 5, -2),
                        ("after", 5, 3),
                    ]
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_listener_can_rewrite_a_zero_point_grant() {
        struct GiveOne;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for GiveOne {
            fn player_xp_change(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::PlayerXpChangeEvent<'_>,
            ) -> mod_api::Flow {
                event.amount = 1;
                mod_api::Flow::Continue
            }
        }
        let mods = vec![mod_api::ModEntry::new("give-one", Box::new(GiveOne))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let player = acting_player(game);
                game.lan_acting_name = Some("ScoreProbe".into());
                game.player_score = 0;
                assert!(player.give_experience_points(game, 0));
                assert_eq!(game.player_score, 1);
                assert_eq!(game.experience.total, 1);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_stale_handle_gives_nothing() {
        with_game(|game| {
            let player = acting_player(game);
            let stale = mod_api::__private::game::player_ref(
                EntityRef::new(WorldEpoch::new(u64::MAX), 7),
                player.uuid(),
            );
            assert!(!game.player_give_experience_points(stale, 10));
            assert!(!game.player_give_levels(stale, 1));
            assert!(!game.player_set_total_experience(stale, 10));
            assert_eq!(game.experience.total, 0);
            assert_eq!(game.experience.level, 0);
        });
    }
}
