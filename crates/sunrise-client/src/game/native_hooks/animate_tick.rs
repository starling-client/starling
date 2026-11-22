//! `Block.animateTick` for mod blocks: the parts of the blocks `ClientLevel.animateTick` picks,
//! called inside the pick.
use super::*;
use crate::game::api_values::block_state::block_listens;
use std::cell::RefCell;
use std::rc::Rc;
use sunrise_core::random::LegacyRandom;

/// The level's random (`ClientLevel.random`) while a client Part holds it as
/// `event.random`: one stream for the part's draws and the game's own draws during the call
/// (`add_particle`'s limiter roll, `Game::with_lent_level_random`), in call order.
pub(in crate::game) type LentRandom = Rc<RefCell<mod_api::LegacyRandomSource>>;

/// `event.random`: draws from the lent level random.
struct LevelRandom(LentRandom);

impl mod_api::RandomSource for LevelRandom {
    fn next_int(&mut self) -> i32 {
        self.0.borrow_mut().next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.0.borrow_mut().next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.0.borrow_mut().next_long()
    }
    fn next_boolean(&mut self) -> bool {
        self.0.borrow_mut().next_boolean()
    }
    fn next_float(&mut self) -> f32 {
        self.0.borrow_mut().next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.0.borrow_mut().next_double()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.0.borrow_mut().next_gaussian()
    }
}

impl Game<'_> {
    /// Whether `state`'s block has a part answering `animate_tick`: the per-block table's
    /// answer, no dispatch. The sampler asks it after each loaded pick's block effects.
    pub(in crate::game) fn animate_part_listens(state: BlockStateId) -> bool {
        block_listens(mod_api::HookId::AnimateTick, state)
    }
    /// Whether a client tick's sampling asks for the block parts' `animate_tick`: some part
    /// answers it and this is a window's client.
    pub(in crate::game) fn mod_animate_parts(&self) -> bool {
        mod_api::__private::has_scoped_hook(mod_api::HookId::AnimateTick) && self.native_window()
    }

    /// Calls the parts of the pick at `pos` (`state`), inside the pick as vanilla's
    /// `doAnimateTick` calls `Block.animateTick`: after the block's own effects, before its
    /// fluid's and the next pick. `event.random` is the level's random itself: the part's
    /// draws and the game's own during the call (`add_particle`'s limiter) move it in order.
    pub(in crate::game) fn mod_animate_tick(&mut self, state: BlockStateId, pos: [i32; 3]) {
        let Some(level) = self.native_client_level() else {
            return;
        };
        self.lend_client_level_random(|game, random| {
            game.native_animate_tick(super::fire::AnimateTickCall {
                level,
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                random,
            });
        });
    }

    /// Lend `ClientLevel.random` to a client Part, sharing the stream with any callbacks
    /// and host effects it calls before returning.
    pub(in crate::game) fn lend_client_level_random<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut dyn mod_api::RandomSource) -> R,
    ) -> R {
        if let Some(lent) = self.lent_level_random.clone() {
            return f(self, &mut LevelRandom(lent));
        }
        let lent: LentRandom = Rc::new(RefCell::new(mod_api::LegacyRandomSource::from(<(
            u64,
            Option<f64>,
        )>::from(
            &*self.particles.level_random(),
        ))));
        self.lent_level_random = Some(lent.clone());
        let out = f(self, &mut LevelRandom(lent.clone()));
        self.lent_level_random = None;
        *self.particles.level_random() =
            LegacyRandom::from(<(u64, Option<f64>)>::from(&*lent.borrow()));
        out
    }

    /// Runs `f`, which draws from the level random (breaking-particle position, `add_particle`'s limiter or
    /// the poofs from `create_fireworks`), on the stream a client Part
    /// holds as `event.random`: the engine's random is that stream for `f`, and
    /// `f`'s draws stay in it.
    pub(in crate::game) fn with_lent_level_random<R>(
        &mut self,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let Some(lent) = self.lent_level_random.clone() else {
            return f(self);
        };
        *self.particles.level_random() =
            LegacyRandom::from(<(u64, Option<f64>)>::from(&*lent.borrow()));
        let out = f(self);
        *lent.borrow_mut() = mod_api::LegacyRandomSource::from(<(u64, Option<f64>)>::from(
            &*self.particles.level_random(),
        ));
        out
    }

    /// A client's random display tick of the mod block `state` at `pos` (`Block.animateTick`),
    /// picked by `ClientLevel.animateTick`'s sampling: its parts add particles and play sounds
    /// through the client host, drawing from the level's `random`. Client-side.
    pub(in crate::game) fn native_animate_tick(&mut self, call: super::fire::AnimateTickCall<'_>) {
        self.fire_animate_tick(call);
    }
}
