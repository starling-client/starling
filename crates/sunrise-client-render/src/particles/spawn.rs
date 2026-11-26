//! Particle creation and server particle-event admission.
use super::*;
use sunrise_protocol::packets::play::LevelParticles;

impl ParticleEngine {
    /// `ClientPacketListener.handleParticleEvent`: spread draws belong to the connection,
    /// independently of the factory and ClientLevel limiter RNGs. A failed callback ends
    /// this packet, unlike a naturally declined or culled spawn.
    pub fn handle_level_particles(
        &mut self,
        random: &mut LegacyRandom,
        level: Option<&ParticleLevel<'_>>,
        packet: &LevelParticles,
        options: &ParticleOptions,
    ) {
        let add = |engine: &mut Self, at, v| {
            engine.add_with(
                level,
                options,
                at,
                v,
                packet.override_limiter,
                packet.always_show,
            )
        };
        if packet.count == 0 {
            add(
                self,
                packet.pos,
                std::array::from_fn(|i| f64::from(packet.max_speed[i] * packet.dist[i])),
            );
            return;
        }
        let (dist, speed) = (packet.dist.map(f64::from), packet.max_speed.map(f64::from));
        for _ in 0..packet.count {
            let (at, v) = spread(random, packet.randomization, packet.pos, dist, speed);
            if !add(self, at, v) {
                break;
            }
        }
    }

    /// `ClientPacketListener.handleParticleEvent` for `ServerLevel.sendParticles`.
    pub fn send_particles(
        &mut self,
        options: &ParticleOptions,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: u32,
        level: Option<&ParticleLevel<'_>>,
    ) {
        if count == 0 {
            let v = [speed * spread[0], speed * spread[1], speed * spread[2]];
            self.add_with(level, options, at, v, false, false);
            return;
        }
        for _ in 0..count {
            let r = &mut self.random;
            let p = [
                at[0] + r.next_gaussian() * spread[0],
                at[1] + r.next_gaussian() * spread[1],
                at[2] + r.next_gaussian() * spread[2],
            ];
            let v = [
                r.next_gaussian() * speed,
                r.next_gaussian() * speed,
                r.next_gaussian() * speed,
            ];
            self.add_with(level, options, p, v, false, false);
        }
    }
    /// `ParticleEngine.createParticle`: always spawned, but `ParticleEngine.add`
    /// drops a spore blossom air particle once `ParticleLimit.SPORE_BLOSSOM` is full.
    pub fn create(&mut self, options: &ParticleOptions, p: [f64; 3], v: [f64; 3]) {
        self.create_with_level(None, options, p, v);
    }

    /// Create a particle directly, with the level used by a typed factory when present.
    /// False means a factory failed; an intentionally empty result is not a failure.
    pub fn create_with_level(
        &mut self,
        level: Option<&ParticleLevel<'_>>,
        options: &ParticleOptions,
        p: [f64; 3],
        v: [f64; 3],
    ) -> bool {
        if let Some(requests) = &mut self.created_requests {
            requests.push((options.clone(), p, v));
            return true;
        }
        let seed = self.seeds.next_long();
        let made = match &options.typed_particle {
            Some((value, _)) => {
                let Some(level) = level else { return true };
                match self.make_typed_particle(level, value, p, v, seed) {
                    Ok(particle) => particle,
                    Err(error) => {
                        log::warn!("Could not spawn particle effect: {error}");
                        return false;
                    }
                }
            }
            None => self.make(options, p, v, seed),
        };
        if let Some(mut particle) = made {
            *self.made.entry(options.ty.name()).or_default() += 1;
            // Resource packs' Polytone particle modifiers (#11486), once as it spawns.
            if let Some(registry) = &self.polytone
                && registry.modifies_particles()
            {
                let id = match &options.mod_particle {
                    Some(name) => name.clone(),
                    None => format!("minecraft:{}", options.ty.name()).into(),
                };
                polytone::modify(registry, &id, options.source.as_deref(), &mut particle);
            }
            // The provider has drawn, so rejecting here keeps vanilla's RNG order.
            // shortcut: counts live plus queued particles by scanning instead of
            // vanilla's counted map; use a counter if spore spawning shows in profiles.
            if particle.ty == ParticleType::SporeBlossomAir
                && self.particles.len() + self.pending.len() >= SPORE_BLOSSOM_LIMIT
                && self
                    .particles
                    .iter()
                    .chain(self.pending.iter().filter_map(PendingParticle::ordinary))
                    .filter(|q| q.ty == ParticleType::SporeBlossomAir)
                    .count()
                    >= SPORE_BLOSSOM_LIMIT
            {
                return true;
            }
            self.push(particle);
        }
        true
    }
}

/// `Mth.nextDouble(random, min, max)`.
pub(super) fn next_double_between(random: &mut impl RandomSource, min: f64, max: f64) -> f64 {
    if min >= max {
        min
    } else {
        random.next_double() * (max - min) + min
    }
}

/// One of a packet's `count` particles (`handleParticleEvent`): its position
/// and speed. DEFAULT (0) draws both from a Gaussian; ALTERNATIVE (1) and
/// ALTERNATIVE_WITH_SPEED (2) spread over `[0, dist)`, the first keeping
/// the speeds as they are, the second scaling each by `nextDouble`.
fn spread(
    r: &mut impl RandomSource,
    randomization: i32,
    pos: [f64; 3],
    dist: [f64; 3],
    speed: [f64; 3],
) -> ([f64; 3], [f64; 3]) {
    if matches!(randomization, 1 | 2) {
        let at = [0, 1, 2].map(|k| pos[k] + r.next_double() * dist[k]);
        let v = if randomization == 2 {
            [0, 1, 2].map(|k| speed[k] * r.next_double())
        } else {
            speed
        };
        (at, v)
    } else {
        let at = [0, 1, 2].map(|k| pos[k] + r.next_gaussian() * dist[k]);
        (at, [0, 1, 2].map(|k| r.next_gaussian() * speed[k]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Nether's evaporation smoke (ALTERNATIVE, dist 1, speed 0) lands
    /// inside the block from its corner and stands still; DEFAULT spreads
    /// both ways.
    #[test]
    fn alternative_particles_spread_from_the_corner_over_dist() {
        let mut r = LegacyRandom::new(1);
        let mut below = false;
        for _ in 0..200 {
            let (at, v) = spread(&mut r, 1, [3.0, 64.0, -7.0], [1.0; 3], [0.0; 3]);
            assert!((3.0..4.0).contains(&at[0]), "{at:?}");
            assert!((64.0..65.0).contains(&at[1]), "{at:?}");
            assert!((-7.0..-6.0).contains(&at[2]), "{at:?}");
            assert_eq!(v, [0.0; 3]);
            let (_, v) = spread(&mut r, 2, [0.0; 3], [0.0; 3], [0.5, 0.0, 2.0]);
            assert!((0.0..0.5).contains(&v[0]) && v[1] == 0.0 && (0.0..2.0).contains(&v[2]));
            let (at, _) = spread(&mut r, 0, [0.0; 3], [1.0; 3], [0.0; 3]);
            below |= at.iter().any(|&c| c < 0.0);
        }
        assert!(below, "DEFAULT is a Gaussian around the position");
    }
}
