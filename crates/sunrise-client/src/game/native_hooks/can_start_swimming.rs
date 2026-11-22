use super::*;
use mod_api::SwimmingCheck;
use sunrise_client::{motion::SwimEntry, sprint};

impl Game<'_> {
    /// The first answer replaces the shared player swim predicate at this call site.
    pub(in crate::game) fn native_can_start_swimming(
        &self,
        player: PlayerRef,
        side: LogicalSide,
        check: mod_api::SwimmingCheck,
        can_start_swimming: &mut Option<bool>,
    ) {
        self.fire_can_start_swimming(player, side, check, can_start_swimming);
    }

    pub(in crate::game) fn has_swim_eligibility_listener(&self) -> bool {
        mod_api::__private::listened(mod_api::HookId::CanStartSwimming)
    }

    fn ask_swim_eligibility(&self, check: SwimmingCheck) -> Option<bool> {
        let (player, side) = if self.remote.is_some() {
            (self.local_player()?, LogicalSide::Client)
        } else {
            let uuid = self.lan_acting_uuid.or_else(|| {
                self.level
                    .player_uuid
                    .map(sunrise_client::lan::playerdata::ints_uuid)
            })?;
            (self.player(uuid)?, LogicalSide::Server)
        };
        if Some(player.entity().id()) != self.player.as_ref().map(|p| p.entity_id as u32) {
            return None;
        }
        let mut answer = None;
        self.native_can_start_swimming(player, side, check, &mut answer);
        answer
    }

    pub(in crate::game) fn ask_swim_entry(
        &self,
        step: Option<SwimEntry>,
    ) -> Option<(SwimEntry, bool)> {
        let step = step?;
        self.ask_swim_eligibility(SwimmingCheck::Entry)
            .map(|answer| (step, answer))
    }

    /// `LocalPlayer.isSprintingPossible` asks about shallow water only once
    /// its earlier checks pass. A sprint setter can precede a second query.
    pub(in crate::game) fn player_sprint_tick(
        &mut self,
        keys: sprint::Keys,
        window: i32,
        vehicle_sprint: Option<(bool, bool)>,
    ) -> sunrise_client::Result<()> {
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        if !mod_api::__private::listened(mod_api::HookId::CanStartSwimming) {
            return match vehicle_sprint {
                Some((can_sprint, local)) => {
                    player.sprint_input_tick_riding(keys, window, can_sprint, local)
                }
                None => player.sprint_input_tick_with_window(keys, window),
            };
        }
        let mut state = player.sprint_input;
        let mut conditions = player.sprint_conditions();
        if let Some((can_sprint, local)) = vehicle_sprint {
            conditions.passenger = true;
            conditions.vehicle_can_sprint = can_sprint;
            conditions.vehicle_local = local;
        }
        let result = state.tick_with_context(
            keys,
            conditions,
            window,
            self,
            |game, vanilla| {
                // Only water has a swimmable fluid type in the current fluid table.
                vanilla
                    || (game
                        .player
                        .as_ref()
                        .is_some_and(|p| p.fluid_height(sunrise_client::motion::WATER) > 0.0)
                        && game.ask_swim_eligibility(SwimmingCheck::ShallowWater) == Some(false))
            },
            |game, value| {
                game.player
                    .as_mut()
                    .expect("sprint owns the player")
                    .set_sprinting(value)
            },
        );
        self.player
            .as_mut()
            .expect("sprint owns the player")
            .sprint_input = state;
        result
    }
}
