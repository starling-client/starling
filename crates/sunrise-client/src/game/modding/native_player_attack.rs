//! A player's attack for native server mods, through a `PlayerRef`:
//! `Player.getAttackStrengthScale(partial)`, the charge `Player.attack` reads
//! at its start (`attack_entity` fires before that, and before the swing
//! resets the ticker), the ticker itself, and `Player.attack` on a target the
//! mod chooses. Reads return None for a stale handle or a player who isn't
//! online here; a resting guest reads its saved body.
use super::Game;
use super::lan_actions::Struck;
use super::native_server::{Acting, PlayerRead};
use mod_api::__private::game::Fields;
use mod_api::{ActionRefused, AttackRefused, EntityRef, PlayerRef};
use sunrise_client::attack_strength::{BASE_ATTACK_SPEED, delay};

impl Game<'_> {
    /// `Player.getAttackStrengthScale(partial)` for `player`: a server's player, or on a client
    /// the local player alone (#5691).
    /// Returns `None` for a non-finite partial tick; finite partials are not restricted.
    pub(in crate::game) fn player_attack_strength_scale(
        &self,
        player: PlayerRef,
        partial: f32,
    ) -> Option<f32> {
        if !partial.is_finite() {
            return None;
        }
        Some(match self.player_read_both(player)? {
            // On a client, the ticker the crosshair's attack indicator reads (`hand_strength`):
            // `attack_ticks` only counts in a world this game runs itself.
            PlayerRead::Acting { .. } if !self.is_logical_server() => self
                .hand_strength
                .scale(partial, self.main_hand_attack_speed()),
            PlayerRead::Acting { .. } => return self.attack_strength_scale_at(partial),
            PlayerRead::Guest(guest) => {
                let (ticks, speed) = self.guest_attack_charge(guest)?;
                ((ticks as f32 + partial) / delay(speed)).clamp(0.0, 1.0)
            }
        })
    }

    /// `Player.getCurrentItemAttackStrengthDelay()` for `player`: a server's player, or on a
    /// client the local player alone (#5691).
    pub(in crate::game) fn player_current_item_attack_strength_delay(
        &self,
        player: PlayerRef,
    ) -> Option<f32> {
        Some(delay(match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => self.main_hand_attack_speed(),
            PlayerRead::Guest(guest) => self.guest_attack_charge(guest)?.1,
        }))
    }

    /// `LivingEntity.attackStrengthTicker` of a server's player: the one in place, or a guest's
    /// between its turns.
    pub(in crate::game) fn player_attack_strength_ticker(&self, player: PlayerRef) -> Option<i32> {
        if !self.is_logical_server() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.attack_ticks,
            PlayerRead::Guest(guest) => self.guest_attack_charge(guest)?.0,
        })
    }

    /// Sets a server's player's `LivingEntity.attackStrengthTicker`, dead or alive, as given.
    pub(in crate::game) fn player_set_attack_strength_ticker(
        &mut self,
        player: PlayerRef,
        ticks: i32,
    ) -> bool {
        self.native_as_resident_player(player, |game| game.attack_ticks = ticks)
    }

    /// `Player.attack(target)` for a connected or fake player, from its start and without the
    /// packet's checks or swing ([`Game::guest_player_attack`]). True once it ran past
    /// `cannotAttack`.
    pub(in crate::game) fn player_attack(
        &mut self,
        player: PlayerRef,
        target: EntityRef,
    ) -> Result<bool, AttackRefused> {
        let epoch = self.native_lifecycle.epoch();
        let me = player.entity();
        if !self.is_logical_server() || epoch != Some(me.epoch()) || epoch != Some(target.epoch()) {
            return Err(AttackRefused::Stale);
        }
        let found = self.lan.as_ref().and_then(|host| {
            let gi = host
                .hub
                .guests
                .identity_index(player.fields().uuid, me.id())?;
            let guest = host.hub.guests.get(gi)?;
            (guest.placed && guest.left_reason.is_empty()).then_some((gi, guest.conn))
        });
        let Some((gi, conn)) = found else {
            // Not in the hub's list as itself: the player whose turn runs now, or gone.
            return Err(match self.player_handle_read(player) {
                Some(_) => AttackRefused::Busy,
                None => AttackRefused::Stale,
            });
        };
        if !self.lan_guest_swaps.is_empty()
            || !(self.transactions.is_empty() || self.in_level_tick_boundary_alone())
            || self.fake_action.acting()
        {
            return Err(AttackRefused::Busy);
        }
        let (Some(from), Some(to)) = (self.entity_level(me), self.entity_level(target)) else {
            return Err(AttackRefused::Stale);
        };
        if from != to || !self.guest_in_place(gi) {
            return Err(AttackRefused::OtherLevel);
        }
        if target.id() == me.id() {
            return Err(AttackRefused::Unsupported);
        }
        let id = i32::try_from(target.id()).map_err(|_| AttackRefused::Stale)?;
        // As a fake player's `perform_action`, beside a level tick listener's transaction.
        let struck = self.beside_listener_journal(|game| {
            let _acting = Acting::start_unswung(&game.fake_action);
            game.guest_player_attack(conn, gi, id, false)
        });
        match struck {
            Ok(struck) => Ok(struck != Struck::Stopped),
            // No body Sunrise strikes along the player's aim.
            Err(ActionRefused::InvalidTarget) => Err(AttackRefused::Unsupported),
            Err(_) => Err(AttackRefused::Stale),
        }
    }

    /// A guest between turns: its attack ticker and its attack speed with the main-hand item of
    /// its last tick.
    fn guest_attack_charge(&self, guest: &sunrise_client::lan::guest::Guest) -> Option<(i32, f64)> {
        let (ticks, item) = self
            .lan
            .as_ref()?
            .play
            .get(&guest.conn)
            .map_or((0, None), |play| (play.attack_ticks(), play.attack_item()));
        Some((ticks, guest_attack_speed(self, guest, item)))
    }
}

/// `Game.main_hand_attack_speed` for a guest between turns: its saved body's
/// attack speed base, the main-hand item of its last tick, and its effects'
/// modifiers.
fn guest_attack_speed(
    game: &Game<'_>,
    guest: &sunrise_client::lan::guest::Guest,
    item: Option<&str>,
) -> f64 {
    let base = guest
        .motion
        .as_ref()
        .and_then(|player| {
            let key = player.attributes.key("minecraft:attack_speed")?;
            let handle = player.attributes.lookup(Some(&key))?;
            player
                .attributes
                .instance(&handle)
                .ok()
                .map(|value| value.base_value())
        })
        .unwrap_or(BASE_ATTACK_SPEED);
    let value = base
        + item
            .and_then(|id| game.items().get(id))
            .map_or(0.0, |info| f64::from(info.attack_speed));
    game.sanitize_attack_speed(
        guest
            .effects
            .modifiers()
            .into_iter()
            .filter(|modifier| modifier.0 == "minecraft:attack_speed")
            .fold(value, |value, modifier| value * (1.0 + modifier.2)),
    )
}

#[cfg(test)]
mod tests {
    use super::Game;
    use mod_api::EntityRef;
    use sunrise_client::items::ItemStack;

    /// A client reads its own player's attack charge and full-charge delay from the ticker and
    /// main-hand attack speed the crosshair's attack indicator reads (`hand_strength`), not the
    /// local world's `attack_ticks`; any other player reads None (#5691).
    #[test]
    #[ignore = "daily"]
    fn a_client_reads_its_own_players_attack_charge_and_delay() {
        use super::super::native_client_session::tests::logged_client;
        use super::super::native_entity_refs::tests::with_client_game;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        mod_platform::testing::with_mods(Default::default(), || {
            with_client_game(|game| {
                let _server = logged_client(game);
                game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                let local = game.local_player().unwrap();
                let other = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(local.entity().epoch(), 99),
                    99,
                );
                // An empty hand: the base attack speed, 4, so a full charge takes 5 ticks.
                let delay = 5.0;
                // `attack_ticks` (full from the start) is not what a client reads.
                game.attack_ticks = 100;
                for (ticks, scale) in [(0, 0.0), (2, 0.4), (40, 1.0)] {
                    game.hand_strength.ticker = ticks;
                    let host = &*game as &dyn mod_api::ClientHost;
                    assert_eq!(local.current_item_attack_strength_delay(host), Some(delay));
                    assert_eq!(local.attack_strength_scale(host, 0.0), Some(scale));
                    assert_eq!(other.current_item_attack_strength_delay(host), None);
                    assert_eq!(other.attack_strength_scale(host, 0.0), None);
                }
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(local.attack_strength_scale(host, 0.0), Some(1.0));
                assert_eq!(local.attack_strength_scale(host, f32::NAN), None);
            });
        });
    }

    /// A zombie `id` standing at `at`, and its handle.
    pub(in crate::game) fn zombie(game: &mut Game<'_>, id: u32, at: [f64; 3]) -> EntityRef {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let [x, y, z] = at;
        game.mobs.push(sunrise_client::mobs::Mob::new(
            id,
            kind,
            sunrise_core::geometry::Vec3::new(x, y, z),
            0.0,
        ));
        EntityRef::new(game.native_lifecycle.epoch().unwrap(), id)
    }

    fn mob<'a>(game: &'a Game<'_>, id: u32) -> &'a sunrise_client::mobs::Mob {
        game.mobs.iter().find(|m| m.id == id).unwrap()
    }

    /// A connected player's `attack` on a zombie, holding a Knockback II, Fire Aspect diamond
    /// sword while falling, at full charge: `Player.attack`'s critical hit (7 x 1.5 through the
    /// zombie's 2 armour), Fire Aspect's 4 seconds of fire, the push along the player's facing
    /// (+z at yaw 0), one point of durability, and the ticker reset, with no swing, which is
    /// the packet's.
    #[test]
    #[ignore = "daily"]
    fn a_mods_attack_runs_player_attack_with_its_crit_enchantments_and_durability_unswung() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        use super::super::native_server::fake_action_tests::fake;
        if run_fresh_cases(
            "game::native_player_attack::tests::a_mods_attack_runs_player_attack_with_its_crit_enchantments_and_durability_unswung",
            "SUNRISE_PLAYER_ATTACK_CASE",
            &["attack"],
        ) {
            return;
        }
        with_game(|game| {
            let (_, _, conn) = fake(game, None, 0);
            let player = game.player(1).unwrap();
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.enchant("minecraft:knockback", 2);
            sword.enchant("minecraft:fire_aspect", 1);
            let host = game.lan.as_mut().unwrap();
            host.hub.update_guest(conn, |g| {
                g.inventory[g.selected] = Some(sword);
                g.on_ground = false;
            });
            game.as_guest(conn, |g| {
                g.put_equipment_modifiers();
                g.player.as_mut().unwrap().fall_distance = 1.0;
            })
            .unwrap();
            let target = zombie(game, 5000, [0.5, 64.0, 2.0]);
            assert!(player.set_attack_strength_ticker(game, 100));
            assert_eq!(player.attack(game, target), Ok(true));
            let damage = 7.0_f32 * 1.5;
            let armor = (2.0_f32 - damage / 2.0).max(2.0 * 0.2);
            let zombie = mob(game, 5000);
            assert_eq!(zombie.health, 20.0 - damage * (1.0 - armor / 25.0));
            assert_eq!(zombie.fire_ticks, 80);
            assert!(zombie.velocity.z > 0.0 && zombie.velocity.x == 0.0);
            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!(guest.inventory[guest.selected].as_ref().unwrap().damage, 1);
            assert_eq!(player.attack_strength_ticker(game), Some(0));
            assert_eq!(
                game.as_guest(conn, |g| g.swing.current.is_none()),
                Some(true)
            );
        });
    }

    /// One gesture striking two zombies: the ticker read once, set back to that value before
    /// the second `attack`, gives both the same partial charge, `7 * (0.2 + c * c * 0.8)` with
    /// `c = (5 + 0.5) / delay` for a diamond sword; each attack resets it. Any value is kept as
    /// given, a negative one too.
    #[test]
    #[ignore = "daily"]
    fn the_ticker_set_back_between_two_attacks_gives_both_the_same_charge() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        use super::super::native_server::fake_action_tests::fake;
        if run_fresh_cases(
            "game::native_player_attack::tests::the_ticker_set_back_between_two_attacks_gives_both_the_same_charge",
            "SUNRISE_PLAYER_TICKER_CASE",
            &["ticker"],
        ) {
            return;
        }
        with_game(|game| {
            let (_, _, conn) = fake(game, Some("minecraft:diamond_sword"), 0);
            let player = game.player(1).unwrap();
            game.as_guest(conn, |g| g.put_equipment_modifiers())
                .unwrap();
            assert!(player.set_attack_strength_ticker(game, -7));
            assert_eq!(player.attack_strength_ticker(game), Some(-7));
            assert!(player.set_attack_strength_ticker(game, 5));
            let swing = player.attack_strength_ticker(game).unwrap();
            assert_eq!(swing, 5);
            let first = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let second = zombie(game, 5001, [-1.5, 64.0, 0.5]);
            assert_eq!(player.attack(game, first), Ok(true));
            assert_eq!(player.attack_strength_ticker(game), Some(0));
            assert!(player.set_attack_strength_ticker(game, swing));
            assert_eq!(player.attack(game, second), Ok(true));
            // Diamond sword: 4 + (-2.4f32) attack speed; Player.attack samples the ticker at 0.5.
            let delay = (1.0 / (4.0 + f64::from(-2.4f32)) * 20.0) as f32;
            let charge = (5.0 + 0.5) / delay;
            let damage = 7.0 * (0.2 + charge * charge * 0.8);
            let armor = (2.0_f32 - damage / 2.0).max(2.0 * 0.2);
            let hurt = 20.0 - damage * (1.0 - armor / 25.0);
            assert_eq!(
                (mob(game, 5000).health, mob(game, 5001).health),
                (hurt, hurt)
            );
        });
    }
}
