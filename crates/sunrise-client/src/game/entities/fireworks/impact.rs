//! Rocket impacts dispatch only after the vanilla deflection decision.
use super::*;

#[cfg(test)]
thread_local! {
    static ROCKET_IMPACT_PREPARES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Game<'_> {
    /// Let listeners inspect and remove the live rocket, preserving its identity
    /// across the detached tick. Refuse a stale hit after writes.
    pub(super) fn rocket_impact(
        &mut self,
        index: usize,
        r: Rocket,
        hit: ProjectileHit,
    ) -> Option<(Rocket, bool)> {
        if matches!(hit, ProjectileHit::None) || !self.native_hears_impacts() {
            return Some((r, false));
        }
        #[cfg(test)]
        ROCKET_IMPACT_PREPARES.with(|count| count.set(count.get() + 1));
        let (id, join, position, velocity) = (r.id, r.join, r.position, r.velocity);
        let owner = self.rocket_owner(&r);
        let target = self.native_impact_target(hit);
        // The resident owner keeps its slot during the tick: publish the advanced
        // state for the listeners, then read it back as the next tick clone.
        if !self.store_rocket_back(index, r) {
            return None;
        }
        let canceled =
            self.native_separate_projectile_impact(id, "minecraft:firework_rocket", owner, hit);
        let at = self.rockets.locate(join)?;
        if self
            .persist
            .removals
            .contains(&super::super::commands::Target::Rocket(id))
        {
            drop(self.rockets.extract(join)?);
            self.rockets.retire_extracted(id, join);
            return None;
        }
        let r = self.rockets[at].clone();
        let changed = r.position != position
            || r.velocity != velocity
            || target != self.native_impact_target(hit);
        Some((r, canceled || changed))
    }
}

#[cfg(test)]
#[path = "impact_tests.rs"]
mod tests;
