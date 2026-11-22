use super::*;

/// The hook point. It does nothing in Sunrise.
impl Game<'_> {
    /// An entity's breathing this tick (`LivingBreatheEvent`, posted from
    /// `LivingEntity.baseTick` every tick for a living entity with the generic
    /// air logic): `entity` is the player or mob. `can_breathe`, `consume` and
    /// `refill` come in as Sunrise does it (a mod-free game changes nothing); set
    /// them to change this tick. With `can_breathe` the air rises by `refill`, up
    /// to its maximum (`entity_max_air_supply`: 300 for every entity except
    /// dolphins, which have 4800); a dolphin's vanilla `refill` is its maximum
    /// minus its current air. Without breath, air falls by `consume`, and at -20
    /// it is set to 0 and the entity takes 2 drowning damage. Negative amounts
    /// count as 0. Asked on servers for players and mobs. A water animal,
    /// axolotl or nautilus comes in with its own air logic's answer: where it
    /// breathes (water, or rain for an axolotl) `can_breathe` and a `refill` to
    /// its full air; elsewhere no breath and `consume` 1. Running out deals its
    /// own damage (drowning, or drying out).
    ///
    /// The listener has the entity's level and can read its air
    /// (`entity_air_supply`), its equipment and the block it stands in, and
    /// write items, attachments and criteria. What it writes before the air
    /// update is committed with it. The game then applies the answer to the air
    /// the entity had when the listener was called: if the entity has gone, died
    /// or had its air changed in the meantime, it refuses (never recomputes),
    /// the air is left as the listener left it, and the writes of the listener
    /// that go through the transaction (inventory edits through `ItemPlace::edit`)
    /// are rolled back. A criterion a listener granted stays granted: criteria are
    /// granted at once, not through the transaction. Mod effects that this
    /// tick's air update causes (drowning, drying out) are not part of the call.
    pub(in crate::game) fn native_living_breathe(
        &mut self,
        entity: mod_api::LivingRef,
        can_breathe: &mut bool,
        consume: &mut i32,
        refill: &mut i32,
    ) {
        self.fire_living_breathe(entity, can_breathe, consume, refill);
    }
}

impl Game<'_> {
    /// Ask `living_breathe` about the air update `step` decided for the player in
    /// `living`, then check with `unchanged` that what the decision read still
    /// stands. The listener's writes and that check are one transaction: a
    /// refusal (`None`) rolls the listener's transactional writes back and
    /// leaves the air as it is, and the caller applies nothing.
    pub(in crate::game) fn ask_breath(
        &mut self,
        living: mod_api::LivingRef,
        vanilla: sunrise_client::breathing::Breath,
        unchanged: impl FnOnce(&Self) -> bool,
    ) -> Option<sunrise_client::breathing::Breath> {
        let mut breath = vanilla;
        let failures = mod_api::runtime::isolated_failures();
        let answered: Result<(), ()> = mod_api::run_transaction(self, |game, _| {
            game.native_living_breathe(
                living,
                &mut breath.can_breathe,
                &mut breath.consume,
                &mut breath.refill,
            );
            if mod_api::runtime::isolated_failures() == failures && unchanged(game) {
                Ok(())
            } else {
                Err(())
            }
        });
        answered.ok().map(|()| breath)
    }

    /// The acting (guest) player's breathing, the answer to apply to the air
    /// `step` decided on, or `None` when the player is gone or their air changed.
    pub(in crate::game) fn ask_player_breath(
        &mut self,
        step: sunrise_client::breathing::AirStep,
    ) -> Option<sunrise_client::breathing::Breath> {
        let id = self.native_acting_entity();
        let living = self.entity(id).and_then(|entity| self.living(entity))?;
        self.ask_breath(living, step.vanilla, move |game| {
            game.player
                .as_ref()
                .is_some_and(|player| player.air == step.air && player.entity_id as u32 == id)
        })
    }

    /// A paused mob's breathing, as `ask_player_breath` does for a player.
    pub(in crate::game) fn ask_mob_breath(
        &mut self,
        pause: &sunrise_client::mobs::ai::AirPause,
    ) -> Option<sunrise_client::breathing::Breath> {
        let living = self
            .entity(pause.mob)
            .and_then(|entity| self.living(entity))?;
        self.ask_breath(living, pause.step.vanilla, |game| {
            game.mobs.iter().any(|m| pause.still_valid(m))
        })
    }
}
