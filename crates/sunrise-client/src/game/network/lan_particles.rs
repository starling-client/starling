//! Particles a server sends its players and its clients draw:
//! `ClientboundLevelParticlesPacket` (`ServerLevel.sendParticles`, which
//! `/particle` uses) and `ClientboundExplodePacket` (`ServerLevel.explode`).
use super::{Game, explosion_settings::BlastEffects, lan::LanHost, level_sounds::LevelSound};
use mod_api::{SoundSource, client::particles::PositionSource};
use sunrise_client::lan::{Body, containers::within_range, hub::Hub};
use sunrise_client::particles::ParticleLevel;
use sunrise_client::particles::{ExplosionParticle, ParticleOptions, ParticleType};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_protocol::{
    entity_data::{Particle, ParticleData, PositionSource as NetworkPositionSource},
    packets::play::{self, Clientbound as C},
};
use sunrise_world::registry::BlockStateId;

/// An explosion packet a guest's turn put off (`lan_explode`).
pub(super) struct LanExplosion {
    center: [f64; 3],
    small: bool,
    radius: f32,
    block_count: usize,
    pushed: Vec<(u32, [f64; 3])>,
    effects: BlastEffects,
}

/// A particle request awaiting delivery to players in its level.
pub(super) struct QueuedParticle {
    pub dimension: &'static str,
    pub options: ParticleOptions,
    pub state: Option<BlockStateId>,
    pub at: [f64; 3],
    pub spread: [f64; 3],
    /// Per-axis maximum speed used by the packet's ALTERNATIVE randomization.
    pub speed: [f64; 3],
    pub count: i32,
    /// The packet's `RandomizationType` (0 DEFAULT, 1 ALTERNATIVE).
    pub randomization: i32,
}

/// `PowderSnowBlock.entityInside`'s server roll. The branch and both float
/// draws follow Java's `getKnownMovement`/`randomBetween` order.
pub(super) fn powder_snow_velocity(
    known: Vec3,
    random: &mut impl RandomSource,
) -> Option<[f64; 3]> {
    sunrise_client::powder_snow::particle_velocity(known, random)
}

fn send_native_nearby(
    host: &LanHost,
    dimension: &str,
    packet: Vec<u8>,
    sunrise_only: bool,
    except: Option<u32>,
    within_range: impl Fn([f64; 3]) -> bool,
) {
    for guest in host.hub.guests.iter().filter(|g| {
        g.placed && g.dimension == dimension && Some(g.conn) != except && within_range(g.position)
    }) {
        // A vanilla client cannot decode a native mod's particle id; see
        // `Particle::sunrise_only`.
        if sunrise_only && !guest.is_sunrise() {
            continue;
        }
        guest.sender.send(packet.clone());
    }
}

/// Resolve and decode a custom envelope once, before any effect can spawn it.
pub(super) fn decode_mod_particle(name: &str, bytes: &[u8]) -> Option<ParticleOptions> {
    match mod_api::__private::decode_particle_stream(name, bytes) {
        Ok(Some(value)) => ParticleOptions::typed(value),
        Ok(None) => Some(ParticleOptions::named(name)),
        Err(error) => {
            log::warn!("Cannot decode particle {name}: {error}");
            None
        }
    }
}

/// A protocol or mob-event listener as the API's plain particle data.
pub(super) fn particle_position_source(source: &NetworkPositionSource) -> PositionSource {
    match source {
        NetworkPositionSource::Block(pos) => PositionSource::Block(*pos),
        NetworkPositionSource::Entity { id, y_offset } => PositionSource::Entity {
            id: *id,
            y_offset: *y_offset,
        },
    }
}

/// An ARGB colour as the options' RGBA.
fn rgba(argb: i32) -> [f32; 4] {
    let c = |shift: i32| ((argb >> shift) & 255) as f32 / 255.0;
    [c(16), c(8), c(0), c(24)]
}
fn rgb(c: [f32; 4]) -> [f32; 3] {
    [c[0], c[1], c[2]]
}

/// `ServerLevel.sendParticles` of one soul particle moving by `velocity`
/// (Soul Speed's `spawn_particles`): sent to the players within 32 blocks.
pub(super) fn send_soul_particle(host: &super::lan::LanHost, at: [f64; 3], velocity: [f64; 3]) {
    let Some(id) = host
        .hub
        .registry_id("minecraft:particle_type", "minecraft:soul")
    else {
        return;
    };
    let packet = C::LevelParticles(play::LevelParticles {
        particle: Particle { id, data: None },
        override_limiter: false,
        always_show: false,
        pos: at,
        dist: velocity.map(|v| v as f32),
        max_speed: [1.0; 3],
        count: 0,
        randomization: 0,
    });
    for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
        let distance: f64 = (0..3)
            .map(|k| (g.position[k].floor() + 0.5 - at[k]).powi(2))
            .sum();
        if distance < 32.0 * 32.0 {
            g.send(&packet);
        }
    }
}

impl Game<'_> {
    /// A snowflake from a vehicle (`PowderSnowBlock.entityInside`), to every player around.
    pub(super) fn lan_snowflake(&self, at: [f64; 3], velocity: [f64; 3]) {
        if let Some(host) = self.lan.as_ref() {
            send_snowflake_particle(host, at, velocity);
        }
    }
}

/// `PowderSnowBlock.entityInside`'s `ServerLevel.sendParticles` of one
/// snowflake (`count` 1, no spread, `RandomizationType.ALTERNATIVE`).
pub(super) fn send_snowflake_particle(
    host: &super::lan::LanHost,
    at: [f64; 3],
    velocity: [f64; 3],
) {
    let Some(id) = host
        .hub
        .registry_id("minecraft:particle_type", "minecraft:snowflake")
    else {
        return;
    };
    let packet = C::LevelParticles(play::LevelParticles {
        particle: Particle { id, data: None },
        override_limiter: false,
        always_show: false,
        pos: at,
        dist: [0.0; 3],
        max_speed: velocity.map(|v| v as f32),
        count: 1,
        randomization: 1,
    });
    for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
        let distance: f64 = (0..3)
            .map(|k| (g.position[k].floor() + 0.5 - at[k]).powi(2))
            .sum();
        if distance < 32.0 * 32.0 {
            g.send(&packet);
        }
    }
}

/// `ServerLevel.sendParticles`: sends a `ClientboundLevelParticlesPacket` to each player within 32 blocks.
pub(super) fn send_hub_level_particles(
    hub: &Hub,
    options: &ParticleOptions,
    at: [f64; 3],
    spread: [f64; 3],
    speed: f64,
    count: u32,
    state: Option<BlockStateId>,
) {
    let Some(particle) = hub.particle(options, state) else {
        return;
    };
    let sunrise_only = particle.sunrise_only();
    let packet = C::LevelParticles(play::LevelParticles {
        particle,
        override_limiter: false,
        always_show: false,
        pos: at,
        dist: spread.map(|d| d as f32),
        max_speed: [speed as f32; 3],
        count: count as i32,
        randomization: 0,
    });
    let mut recipients = hub
        .guests
        .iter()
        .filter(|guest| {
            if !hub.here(guest)
                || !guest.participates_online()
                || sunrise_only && !guest.is_sunrise()
            {
                return false;
            }
            // ServerLevel compares the player's block centre, strictly within 32.
            let distance: f64 = (0..3)
                .map(|k| (guest.position[k].floor() + 0.5 - at[k]).powi(2))
                .sum();
            distance < 32.0 * 32.0
        })
        .peekable();
    let Some(first) = recipients.next() else {
        return;
    };
    let body = match packet.try_encode() {
        Ok(body) => body,
        Err(error) => {
            eprintln!("[lan] send refused: {error}");
            return;
        }
    };
    if recipients.peek().is_none() {
        first.sender.send(body);
        return;
    }
    let shared = Body::shared(body);
    for guest in std::iter::once(first).chain(recipients) {
        guest.sender.send_many([shared.clone()]);
    }
}

impl Game<'_> {
    /// `SonicBoom.tick` uses `ServerLevel.sendParticles` for each ray point.
    /// Keep the local event's rendering; connected players receive one at rest.
    pub(super) fn lan_sonic_boom(&self, options: &ParticleOptions, at: [f64; 3]) {
        let (Some(host), Some(particle)) = (&self.lan, self.network_particle(options, None)) else {
            return;
        };
        let packet = C::LevelParticles(play::LevelParticles {
            particle,
            override_limiter: false,
            always_show: false,
            pos: at,
            dist: [0.0; 3],
            max_speed: [0.0; 3],
            count: 0,
            randomization: 0,
        });
        for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
            // ServerLevel compares the player's block centre, strictly within 32.
            let distance: f64 = (0..3)
                .map(|k| (g.position[k].floor() + 0.5 - at[k]).powi(2))
                .sum();
            if distance < 32.0 * 32.0 {
                g.send(&packet);
            }
        }
    }

    /// `ServerLevel.sendParticles` of a mob's particles: a packet to each
    /// player within 32 blocks. The window draws its own copy. `state` is the
    /// block of a block particle.
    pub(super) fn lan_level_particles(
        &self,
        options: &ParticleOptions,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: u32,
        state: Option<BlockStateId>,
    ) {
        let Some(hub) = self.lan.as_ref().map(|h| &h.hub) else {
            return;
        };
        send_hub_level_particles(hub, options, at, spread, speed, count, state);
    }

    /// `LivingEntity.checkFallDamage`'s `ServerLevel.sendParticles` of the
    /// landing block's crumbs: drawn in this window, when it has one, and
    /// sent to each player within 32 blocks.
    pub(super) fn landing_dust(&mut self, state: BlockStateId, at: Vec3, count: i32, speed: f64) {
        let at = [at.x, at.y, at.z];
        if self.serving.is_none() && self.options.dedicated.is_none() {
            let block = [
                at[0].floor() as i32,
                (at[1] - 0.2).floor() as i32,
                at[2].floor() as i32,
            ];
            if let Some((sprite, tint)) = self.particle_sprite(state, block) {
                self.particles.send_particles(
                    &ParticleOptions::block(ParticleType::Block, sprite, tint)
                        .with_source(&self.res.facts().get(state).name),
                    at,
                    [0.0; 3],
                    speed,
                    count as u32,
                    None,
                );
            }
        }
        let Some(hub) = self.lan.as_ref().map(|h| &h.hub) else {
            return;
        };
        let Some(particle) =
            hub.particle(&ParticleOptions::simple(ParticleType::Block), Some(state))
        else {
            return;
        };
        let packet = C::LevelParticles(play::LevelParticles {
            particle,
            override_limiter: false,
            always_show: false,
            pos: at,
            dist: [0.0; 3],
            max_speed: [speed as f32; 3],
            count,
            randomization: 0,
        });
        for g in hub.guests.iter().filter(|g| hub.here(g)) {
            // ServerLevel compares the player's block centre, strictly within 32.
            let distance: f64 = (0..3)
                .map(|k| (g.position[k].floor() + 0.5 - at[k]).powi(2))
                .sum();
            if distance < 32.0 * 32.0 {
                g.send(&packet);
            }
        }
    }

    /// Shared block callback for any entity family that applies block effects.
    pub(super) fn entity_powder_snow_particles(
        &mut self,
        previous: Vec3,
        position: Vec3,
        bounds: Aabb,
        known_movement: Vec3,
        living: bool,
    ) {
        // `particle_velocity` refuses no horizontal movement before any draw,
        // so a resting entity skips the sweep (most items, orbs and carts).
        if self.remote.is_some() || (known_movement.x == 0.0 && known_movement.z == 0.0) {
            return;
        }
        let snow = sunrise_client::powder_snow::contacts(previous, position, bounds, living, |p| {
            &*self.res.facts().get(self.world.block(p.x, p.y, p.z)).name == "minecraft:powder_snow"
        });
        for block in snow {
            self.powder_snow_particle(block, position, known_movement);
        }
    }

    /// `PowderSnowBlock.entityInside` for an entity with known movement.
    pub(super) fn powder_snow_particle(
        &mut self,
        block: BlockPos,
        position: Vec3,
        known_movement: Vec3,
    ) {
        if self.remote.is_some() {
            return;
        }
        let Some(velocity) = powder_snow_velocity(known_movement, &mut self.world.random) else {
            return;
        };
        self.snowflake_particle([position.x, f64::from(block.y + 1), position.z], velocity);
    }

    /// Locally draw the level's own particle, then send it to LAN players.
    pub(super) fn snowflake_particle(&mut self, at: [f64; 3], velocity: [f64; 3]) {
        let options = ParticleOptions::simple(ParticleType::Snowflake);
        if self.options.execution_role.is_client() {
            self.particles.add(&options, at, velocity);
        }
        self.lan_send_particles(QueuedParticle {
            dimension: self.dim.id(),
            options,
            state: None,
            at,
            spread: [0.0; 3],
            speed: velocity,
            count: 1,
            randomization: 1,
        });
    }

    /// `ParticleTypes.STREAM_CODEC`'s value for `options`: the type's
    /// network id and the data it carries (`state` for the block particles).
    /// None while not serving, or for a particle whose data isn't sent from
    /// here (items, trails).
    pub(super) fn network_particle(
        &self,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
    ) -> Option<Particle> {
        self.lan.as_ref()?.hub.particle(options, state)
    }

    /// The options a server's particle stands for (None for a type or data
    /// this client doesn't draw).
    pub(super) fn received_particle(
        &mut self,
        p: &Particle,
        at: [f64; 3],
    ) -> Option<ParticleOptions> {
        if let Some(ParticleData::Mod { name, options }) = &p.data {
            return decode_mod_particle(name, options);
        }
        let name = self
            .remote
            .as_ref()?
            .builtin
            .get("minecraft:particle_type")?
            .name(p.id)?
            .to_owned();
        let ty = ParticleType::from_name(&name)?;
        Some(match &p.data {
            None => ParticleOptions::simple(ty),
            Some(ParticleData::Item(stack)) => {
                let remote = self.remote.as_ref()?;
                let stack = sunrise_client::item_sync::from_network(
                    stack,
                    &remote.builtin,
                    &remote.registries,
                );
                self.item_particle_options(&stack)?
            }
            Some(ParticleData::State(s)) => self.block_provider_options(
                ty,
                BlockStateId(*s as u32),
                at.map(|v| v.floor() as i32),
            )?,
            Some(ParticleData::ColorScale(c, scale)) => {
                ParticleOptions::dust(rgb(rgba(*c)), *scale)
            }
            Some(ParticleData::ColorScale2(from, to, scale)) => {
                ParticleOptions::dust_transition(rgb(rgba(*from)), rgb(rgba(*to)), *scale)
            }
            Some(ParticleData::ColorPower(c, power)) => ParticleOptions {
                scale: *power,
                ..ParticleOptions::colored(ty, [rgba(*c)[0], rgba(*c)[1], rgba(*c)[2], 1.0])
            },
            Some(ParticleData::Color(c)) => ParticleOptions::colored(ty, rgba(*c)),
            Some(ParticleData::Scalar(v)) if ty == ParticleType::SculkCharge => {
                ParticleOptions::sculk_charge(*v)
            }
            Some(ParticleData::Scalar(v)) => ParticleOptions {
                scale: *v,
                ..ParticleOptions::simple(ty)
            },
            Some(ParticleData::Delay(d)) => ParticleOptions::shriek(*d),
            Some(ParticleData::Geyser(water_blocks)) => ParticleOptions::geyser(ty, *water_blocks),
            Some(ParticleData::GeyserBase(water_blocks, burst_impulse_base)) => {
                ParticleOptions::geyser_base(ty, *water_blocks, *burst_impulse_base)
            }
            Some(ParticleData::Trail(target, color, duration)) => {
                ParticleOptions::trail(*target, *color as u32 & 0xFF_FFFF, *duration)
            }
            Some(ParticleData::Vibration { source, ticks }) => ParticleOptions::vibration(
                particle_position_source(source),
                self.vibration_destination(source)?,
                *ticks,
            ),
            Some(_) => return None,
        })
    }

    /// `PositionSource.getPosition`: a block's centre, or an entity's feet
    /// raised by the option's offset. An entity this client does not have is
    /// no destination at all: vanilla makes the particle and removes it on its
    /// first tick, so nothing of it is ever drawn.
    fn vibration_destination(&self, source: &NetworkPositionSource) -> Option<[f64; 3]> {
        match source {
            NetworkPositionSource::Block([x, y, z]) => Some([
                f64::from(*x) + 0.5,
                f64::from(*y) + 0.5,
                f64::from(*z) + 0.5,
            ]),
            NetworkPositionSource::Entity { id, y_offset } => {
                let id = u32::try_from(*id).ok()?;
                self.mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)
                    .map(|m| {
                        [
                            m.position.x,
                            m.position.y + f64::from(*y_offset),
                            m.position.z,
                        ]
                    })
            }
        }
    }

    /// `ClientPacketListener.handleParticleEvent`: no count is one particle
    /// moving by speed times spread; otherwise each is spread and sped up by
    /// gaussians on each axis.
    pub(super) fn remote_level_particles(&mut self, p: &play::LevelParticles) {
        let Some(options) = self.received_particle(&p.particle, p.pos) else {
            return;
        };
        // Combat feel Legacy draws a hit as 1.8.9 did: no sweep, no damage hearts.
        if self.pvp_hides_particle(options.ty) {
            return;
        }
        let Some(remote) = &mut self.remote else {
            return;
        };
        let level = ParticleLevel::new(&self.world, self.res.facts(), self.collision);
        self.particles.handle_level_particles(
            &mut remote.particle_packet_random,
            Some(&level),
            p,
            &options,
        );
    }

    /// `ClientPacketListener.handleExplosion`: the blast's particles (its
    /// block particles as the packet lists them) and its sound, when the
    /// packet carries it, then the player's push.
    pub(super) fn remote_explode(&mut self, p: &play::Explode) {
        if p.play_sound {
            let pitch = (1.0
                + (self.particles.random().next_float() - self.particles.random().next_float())
                    * 0.2)
                * 0.7;
            // The packet's explosionSound: a wind charge's burst, a bed's
            // or anchor's blast, a modded sound.
            let sound = self
                .remote_sound_name(&p.sound)
                .unwrap_or_else(|| "minecraft:entity.generic.explode".to_owned());
            self.play_audio(&sound, Some(p.center), 4.0, pitch);
        }
        if let Some(options) = self.received_particle(&p.particle, p.center) {
            let level = ParticleLevel::new(&self.world, self.res.facts(), self.collision);
            self.particles.add_with(
                Some(&level),
                &options,
                p.center,
                [1.0, 0.0, 0.0],
                false,
                false,
            );
        }
        // A wind charge's burst lists none, and one this client can't draw is left out.
        let block_particles: Vec<_> = p
            .block_particles
            .iter()
            .filter_map(|b| {
                Some(ExplosionParticle {
                    options: self.received_particle(&b.particle, p.center)?,
                    scaling: b.scaling,
                    speed: b.speed,
                    weight: u32::try_from(b.weight).ok()?,
                })
            })
            .collect();
        self.particles.track_explosion(
            p.center,
            p.radius,
            p.block_count.max(0) as usize,
            &block_particles,
        );
        // `Entity.pushFromExplosion` calls `Entity.push`, which rejects a
        // non-finite impulse before even marking the entity for sync.
        if let (Some([x, y, z]), Some(player)) = (p.knockback, self.player.as_mut()) {
            let impulse = Vec3::new(x, y, z);
            if impulse.is_finite() {
                let velocity = player.velocity.add_vector(impulse);
                // `Entity.setDeltaMovement` can reject the sum, but the push
                // still marks the entity for sync.
                if velocity.is_finite() {
                    player.velocity = velocity;
                }
                player.needs_sync = true;
            }
        }
    }

    /// `ServerLevel.sendParticles` for the players a server serves: one
    /// `ClientboundLevelParticlesPacket` to each player in the level whose
    /// block is within 32 blocks of `at`. The window's own player is not
    /// sent it (the caller draws it in this window). While the host holds
    /// the players (a guest's turn through `as_guest`, a host's hit on a
    /// guest) the packet waits for the turn to end so that it reaches every
    /// player, the guest too.
    pub(super) fn lan_send_particles(&mut self, request: QueuedParticle) {
        if self.lan.is_none() {
            if self.serving.is_some() || self.options.dedicated.is_some() {
                self.creatures.lan_particles.push(request);
            }
            return;
        }
        let QueuedParticle {
            dimension,
            options,
            state,
            at,
            spread,
            speed,
            count,
            randomization,
        } = request;
        let Some(particle) = self.network_particle(&options, state) else {
            return;
        };
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let sunrise_only = particle.sunrise_only();
        let body = C::LevelParticles(play::LevelParticles {
            particle,
            override_limiter: false,
            always_show: false,
            pos: at,
            dist: spread.map(|d| d as f32),
            max_speed: speed.map(|v| v as f32),
            count,
            randomization,
        })
        .encode();
        // BlockPos.closerToCenterThan(at, 32): from the centre of the
        // player's block.
        send_native_nearby(host, dimension, body, sunrise_only, None, |position| {
            let d: f64 = (0..3)
                .map(|k| (position[k].floor() + 0.5 - at[k]).powi(2))
                .sum();
            d < 32.0 * 32.0
        });
    }

    /// Resolve through the level-sound hooks in the sound's own level. Sounds emitted from
    /// inside a listener skip re-dispatch, whether still in the listener or queued for a flush.
    /// Preserve the original holder unless a listener replaces its event key.
    pub(super) fn resolve_native_sound(
        &mut self,
        mut sound: super::creatures::NativeSound,
    ) -> Option<super::creatures::NativeSound> {
        if sound.skip_hooks || self.creatures.asking_level_sound {
            return Some(sound);
        }
        let emitter = sound.entity.and_then(|id| u32::try_from(id).ok());
        let mut asked = LevelSound::new(&sound.sound, sound.at, sound.volume, sound.pitch)
            .in_source(
                SoundSource::ALL
                    .get(sound.source as usize)
                    .copied()
                    .unwrap_or(SoundSource::Master),
            );
        asked.emitter = emitter;
        let dim = super::Dim::from_id(sound.dimension);
        let heard =
            self.with_effect_level(dim, |game| game.ask_level_sound(sound.dimension, asked))??;
        let (edited_name, source, volume, pitch) = (
            match heard.name {
                std::borrow::Cow::Borrowed(_) => None,
                std::borrow::Cow::Owned(name) => Some(name),
            },
            heard.source as i32,
            heard.volume,
            heard.pitch,
        );
        if let Some(name) = edited_name
            && name != sound.sound
        {
            sound.sound = name;
            sound.inline_range = None;
        }
        sound.source = source;
        sound.volume = volume;
        sound.pitch = pitch;
        Some(sound)
    }

    /// Pure packet delivery for a native sound that has already passed `resolve_native_sound`
    /// (runs no mod hooks and does not touch `self.creatures.lan_sounds`).
    pub(super) fn send_resolved_native_sound(&self, sound: &super::creatures::NativeSound) {
        let Some(host) = &self.lan else { return };
        let mut packet = host.hub.sound_packet(
            &sound.sound,
            sound.at,
            sound.volume,
            sound.pitch,
            sound.source,
            sound.seed,
        );
        if let Some(range) = sound.inline_range
            && let C::Sound(packet) = &mut packet
        {
            packet.sound = (-1, Some((sound.sound.clone(), range)));
        }
        // `Level.playSound(except, entity, ...)`: the same sound and
        // seed, bound to the entity.
        let packet = match (packet, sound.entity) {
            (C::Sound(s), Some(id)) => C::SoundEntity(play::SoundEntity {
                sound: s.sound,
                source: s.source,
                id,
                volume: s.volume,
                pitch: s.pitch,
                seed: s.seed,
            }),
            (packet, _) => packet,
        }
        .encode();
        let range = sound
            .inline_range
            .flatten()
            .map_or_else(|| f64::from(sound.volume.max(1.0)) * 16.0, f64::from);
        // Wrapped for the level's viewers too: one bound to an entity for those tracking it.
        host.hub
            .send_viewed_sound(sound.dimension, sound.at, range, sound.entity, || {
                packet.clone()
            });
        send_native_nearby(
            host,
            sound.dimension,
            packet,
            false,
            sound.except,
            |position| within_range(sound.at, position, range),
        );
    }

    /// Resolve tick-queued sounds and send them to their level's players and viewers.
    /// The packet uses the registry ID when present or carries the event inline by name.
    pub(super) fn flush_native_sounds(&mut self) {
        if self.lan.is_none() {
            return;
        }
        // Leave sounds queued by callbacks for the next flush.
        for sound in std::mem::take(&mut self.creatures.lan_sounds) {
            if let Some(resolved) = self.resolve_native_sound(sound) {
                self.send_resolved_native_sound(&resolved);
            }
        }
    }

    /// `BucketItem.emptyContents` where water evaporates: `sendParticles(
    /// LARGE_SMOKE, x, y, z, 8, 1, 1, 1, 0, ALTERNATIVE)` from the block's
    /// corner, so each puff lands inside the block. The window draws its own.
    pub(super) fn evaporation_smoke(&mut self, pos: [i32; 3]) {
        let options = ParticleOptions::simple(ParticleType::LargeSmoke);
        let at = pos.map(f64::from);
        for _ in 0..8 {
            let r = self.particles.random();
            let p = [
                at[0] + r.next_double(),
                at[1] + r.next_double(),
                at[2] + r.next_double(),
            ];
            self.particles.add(&options, p, [0.0; 3]);
        }
        self.lan_send_particles(QueuedParticle {
            dimension: self.dim.id(),
            options,
            state: None,
            at,
            spread: [1.0; 3],
            speed: [0.0; 3],
            count: 8,
            randomization: 1,
        });
    }

    /// The particles guests' turns put off.
    pub(super) fn flush_lan_particles(&mut self) {
        for request in std::mem::take(&mut self.creatures.lan_particles) {
            self.lan_send_particles(request);
        }
    }

    /// `ServerLevel.explode`'s packet to each player within 64 blocks, with
    /// the push `pushed` holds for them (`ServerExplosion.hitPlayers`: a
    /// creative player, whom nothing hurt), and the blast's `effects`. The
    /// sound reaches them on its own (the level's sounds), and so does a hurt
    /// player's push (the motion they are sent).
    pub(super) fn lan_explode(
        &mut self,
        center: [f64; 3],
        small: bool,
        radius: f32,
        block_count: usize,
        pushed: Vec<(u32, [f64; 3])>,
        effects: BlastEffects,
    ) {
        // In a guest's turn (their hit on an end crystal or a minecart) the
        // host holds the players (`as_guest`): the packet waits for the turn
        // to end so that it reaches every player, the guest too.
        if self.lan_acting.is_some() && self.lan.is_none() {
            self.creatures.lan_explosions.push(LanExplosion {
                center,
                small,
                radius,
                block_count,
                pushed,
                effects,
            });
            return;
        }
        let packet = self.lan.as_ref().and_then(|lan| {
            explosion_packet(
                &lan.hub,
                center,
                radius,
                block_count,
                effects.emitter(small),
                &effects.sound,
                &effects.block_particles,
            )
        });
        if let Some(packet) = packet {
            self.send_explosion(packet, &pushed);
        }
    }

    /// The explosions guests' turns put off.
    pub(super) fn flush_lan_explosions(&mut self) {
        for e in std::mem::take(&mut self.creatures.lan_explosions) {
            self.lan_explode(
                e.center,
                e.small,
                e.radius,
                e.block_count,
                e.pushed,
                e.effects,
            );
        }
    }

    /// A wind charge's burst (`AbstractWindCharge.explode`): gust emitters,
    /// no block particles, and for each player it pushed the push
    /// (`ServerExplosion.hitPlayers`), which only that client applies, as
    /// nothing hurt them.
    pub(super) fn lan_wind_burst(
        &self,
        center: [f64; 3],
        radius: f32,
        sound: &str,
        block_count: usize,
        pushed: &[(u32, [f64; 3])],
    ) {
        let emitter = ParticleOptions::simple(if radius < 2.0 {
            ParticleType::GustEmitterSmall
        } else {
            ParticleType::GustEmitterLarge
        });
        let packet = self.lan.as_ref().and_then(|lan| {
            explosion_packet(&lan.hub, center, radius, block_count, &emitter, sound, &[])
        });
        if let Some(packet) = packet {
            self.send_explosion(packet, pushed);
        }
    }

    /// `packet` to each player within 64 blocks of its centre, with the push
    /// `pushed` holds for them.
    fn send_explosion(&self, mut packet: play::Explode, pushed: &[(u32, [f64; 3])]) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let center = packet.center;
        let plain = C::Explode(packet.clone()).encode();
        for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
            let d: f64 = (0..3).map(|k| (g.position[k] - center[k]).powi(2)).sum();
            if d >= 4096.0 {
                continue;
            }
            match pushed.iter().find(|(conn, _)| *conn == g.conn) {
                Some((_, push)) => {
                    packet.knockback = Some(*push);
                    g.send(&C::Explode(packet.clone()));
                }
                None => g.sender.send(plain.clone()),
            }
        }
    }
}

/// `ClientboundExplodePacket` without a push, or None while not serving.
/// The sound is the level's, which the players hear already, so it isn't
/// played from the packet.
fn explosion_packet(
    hub: &Hub,
    center: [f64; 3],
    radius: f32,
    block_count: usize,
    emitter: &ParticleOptions,
    sound: &str,
    block_particles: &[ExplosionParticle],
) -> Option<play::Explode> {
    let block_particles = block_particles
        .iter()
        .map(|p| {
            Some(play::ExplosionParticle {
                particle: hub.particle(&p.options, None)?,
                scaling: p.scaling,
                speed: p.speed,
                weight: i32::try_from(p.weight).ok()?,
            })
        })
        .collect::<Option<_>>()?;
    Some(play::Explode {
        center,
        radius,
        block_count: block_count as i32,
        knockback: None,
        particle: hub.particle(emitter, None)?,
        sound: match hub.registry_id("minecraft:sound_event", sound) {
            Some(id) => (id, None),
            None => (-1, Some((sound.into(), None))),
        },
        block_particles,
        play_sound: false,
    })
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use sunrise_client::lan::{
        Joined, Lan, NetConfig, Out, PlayerSender,
        chunks::ChunkEncoder,
        data,
        guest::Guest,
        hub::{Hub, HubConfig},
    };
    use sunrise_client::particles::{ExplosionParticle, ParticleOptions, ParticleType};
    use sunrise_core::random::LegacyRandom;
    use sunrise_protocol::packets::play;

    #[test]
    #[ignore = "daily"]
    fn received_block_particles_follow_the_provider_matrix() {
        use crate::game::{
            in_own_process, native_client_session::tests::logged_client,
            native_entity_refs::tests::with_client_game,
        };
        in_own_process(
            "game::lan_particles::tests::received_block_particles_follow_the_provider_matrix",
            || {
                with_client_game(|game| {
                    let _server = logged_client(game);
                    let types = [
                        ParticleType::Block,
                        ParticleType::DustPillar,
                        ParticleType::BlockCrumble,
                        ParticleType::FallingDust,
                        ParticleType::BlockMarker,
                    ];
                    let registry = game
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:particle_type")
                        .unwrap();
                    let ids =
                        types.map(|ty| registry.id(&format!("minecraft:{}", ty.name())).unwrap());
                    for (name, terrain, dust) in [
                        ("air", false, true),
                        ("cave_air", false, true),
                        ("void_air", false, true),
                        ("moving_piston", false, false),
                        ("barrier", false, false),
                        ("structure_void", false, false),
                        ("light", true, false),
                        ("water", true, false),
                        ("chest", true, true),
                        ("sand", true, true),
                        ("stone", true, true),
                    ] {
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state(&format!("minecraft:{name}"))
                            .unwrap();
                        for ((ty, id), admit) in types
                            .into_iter()
                            .zip(ids)
                            .zip([terrain, terrain, terrain, dust, true])
                        {
                            let result = game.received_particle(
                                &Particle {
                                    id,
                                    data: Some(ParticleData::State(state.0 as i32)),
                                },
                                [0.5, 64.5, 0.5],
                            );
                            assert_eq!(result.is_some(), admit, "{name} for {}", ty.name());
                            if let Some(options) = result {
                                assert_eq!(options.ty, ty);
                                let (sprite, tint) = options.block.unwrap();
                                if ty == ParticleType::BlockMarker {
                                    assert!(!sprite.is_empty());
                                    assert_eq!(tint, 0xFFFFFF);
                                } else if ty == ParticleType::FallingDust {
                                    assert!(
                                        sprite.is_empty(),
                                        "falling dust uses the provider's sprite set"
                                    );
                                    match name {
                                        "sand" => assert_eq!(tint, 0xDBD3A0),
                                        "stone" => assert_eq!(tint, 0x707070),
                                        "air" | "cave_air" | "void_air" => assert_eq!(tint, 0),
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                })
            },
        );
    }

    /// `PowderSnowBlock.entityInside`: vertical-only movement skips the roll;
    /// horizontal movement uses nextBoolean, then X and Z floats in order.
    #[test]
    fn powder_snow_snowflakes_use_the_vanilla_movement_gate_and_random_order() {
        let mut stationary = LegacyRandom::new(1);
        let mut untouched = LegacyRandom::new(1);
        assert_eq!(
            powder_snow_velocity(Vec3::new(0.0, 1.0, 0.0), &mut stationary),
            None
        );
        assert_eq!(stationary.next_int(), untouched.next_int());

        let seed = (0..1000)
            .find(|seed| {
                let mut random = LegacyRandom::new(*seed);
                random.next_boolean()
            })
            .unwrap();
        let mut vanilla = LegacyRandom::new(seed);
        assert!(vanilla.next_boolean());
        let x = (vanilla.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
        let z = (vanilla.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
        let expected = [f64::from(x), f64::from(0.05_f32), f64::from(z)];
        let mut random = LegacyRandom::new(seed);
        assert_eq!(
            powder_snow_velocity(Vec3::new(-0.2, 0.0, 0.0), &mut random),
            Some(expected)
        );
    }

    /// A hub whose `minecraft:particle_type` registry holds `particle_names`
    /// in registry order, plus a near guest at (5, 64, 5) and a far one at
    /// (100, 64, 100).
    pub(in crate::game) fn test_hub(
        label: &str,
        particle_names: Vec<String>,
        supplied_sync: Option<std::sync::Arc<data::SyncData>>,
    ) -> (
        Hub,
        std::sync::mpsc::Receiver<Out>,
        std::sync::mpsc::Receiver<Out>,
    ) {
        let config = NetConfig {
            announce: false,
            listen: false,
            ..NetConfig::loopback(0, label.into(), 2)
        };
        let mut sync = data::SyncData::empty();
        sync.builtin.registries.insert(
            "minecraft:particle_type".to_string(),
            sunrise_protocol::registry::Ids::from_names(particle_names),
        );
        let sync = supplied_sync.unwrap_or_else(|| std::sync::Arc::new(sync));
        let lan = Lan::start_with(sync.clone(), config).unwrap();
        let encoder = ChunkEncoder::new(
            [] as [(std::sync::Arc<str>, bool); 0],
            &sunrise_protocol::registry::Builtin::default(),
            &[],
            &[],
            Default::default(),
        );
        let hub_config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 10,
            simulation_distance: 10,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 20,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        };
        let mut hub = Hub::new(
            lan,
            sync,
            encoder,
            hub_config,
            std::sync::Arc::new(
                crate::game::test_server_load()
                    .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
            ),
        );

        let guest = |conn, uuid, name: &str, at, tx| {
            let mut g = Guest::new(
                Joined {
                    conn,
                    uuid,
                    name: name.into(),
                    properties: Vec::new(),
                    info: sunrise_protocol::packets::configuration::ClientInformation::new(8),
                    brand: "vanilla".into(),
                    sender: PlayerSender::new(tx, std::sync::Arc::default()),
                    latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
                    address: format!("127.0.0.1:{}", 25565 + conn).parse().unwrap(),
                    expected: false,
                },
                1,
                at,
                8,
            );
            g.placed = true;
            g.dimension = "minecraft:overworld".into();
            g
        };
        let (tx_near, rx_near) = std::sync::mpsc::channel();
        hub.guests
            .push(guest(1, 1, "NearPlayer", [5.0, 64.0, 5.0], tx_near));
        let (tx_far, rx_far) = std::sync::mpsc::channel();
        hub.guests
            .push(guest(2, 2, "FarPlayer", [100.0, 64.0, 100.0], tx_far));
        (hub, rx_near, rx_far)
    }

    /// A blast's effects reach `ClientboundExplodePacket` as given: its centre
    /// particle, its weighted block particles and its sound.
    #[test]
    fn an_explosion_packet_carries_the_blasts_particles_and_sound() {
        let names = vec!["minecraft:flame".into(), "minecraft:end_rod".into()];
        let (hub, _near, _far) = test_hub("ExplosionEffectsTest", names, None);
        let block_particles = [ExplosionParticle {
            options: ParticleOptions::simple(ParticleType::EndRod),
            scaling: 0.25,
            speed: 2.0,
            weight: 3,
        }];
        let flame = ParticleOptions::simple(ParticleType::Flame);
        let packet = explosion_packet(
            &hub,
            [1.0, 64.0, 2.0],
            5.0,
            7,
            &flame,
            "test:fuel_tank_burst",
            &block_particles,
        )
        .unwrap();
        assert_eq!(
            (packet.center, packet.radius, packet.block_count),
            ([1.0, 64.0, 2.0], 5.0, 7)
        );
        assert_eq!((packet.particle.id, packet.particle.data), (0, None));
        let [b] = packet.block_particles.as_slice() else {
            panic!("one block particle: {:?}", packet.block_particles);
        };
        assert_eq!(
            (b.particle.id, b.scaling, b.speed, b.weight),
            (1, 0.25, 2.0, 3)
        );
        assert_eq!(
            packet.sound,
            (-1, Some(("test:fuel_tank_burst".to_owned(), None)))
        );
    }

    /// The one `ClientboundLevelParticlesPacket` a guest receives.
    fn level_particles(rx: &std::sync::mpsc::Receiver<Out>) -> play::LevelParticles {
        let out = rx
            .try_recv()
            .expect("guest should receive a LevelParticles packet");
        let body = match out {
            Out::Packet(body) => body,
            _ => panic!("expected Out::Packet"),
        };
        let mut r = sunrise_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        match play::Clientbound::decode(id, r.rest()).unwrap() {
            play::Clientbound::LevelParticles(p) => p,
            other => panic!("expected LevelParticles packet, got {other:?}"),
        }
    }

    thread_local! {
        pub(in crate::game) static PARTICLE_MAP_READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    pub(in crate::game) struct MeasuredParticle;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for MeasuredParticle {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            registrar.register_particle_type(
                "measured",
                false,
                mod_api::ParticleMapCodec::new(
                    |fields, _| match fields.get(&"measure".into()) {
                        Some(mod_api::Tag::Int(value)) => {
                            PARTICLE_MAP_READS.set(PARTICLE_MAP_READS.get() + 1);
                            Ok(*value)
                        }
                        _ => Err("measure must be an integer".into()),
                    },
                    |value, _| {
                        Ok(mod_api::Compound::from([(
                            "measure".into(),
                            mod_api::Tag::Int(*value),
                        )]))
                    },
                ),
                mod_api::ParticleStreamCodec::mapped(
                    |value, _| Ok(value),
                    |value, _| {
                        if *value >= 0 {
                            Ok(*value)
                        } else {
                            Err("negative measure".into())
                        }
                    },
                ),
            );
        }
    }

    #[test]
    fn typed_particle_packet_is_shared_by_spawns_and_kept_from_vanilla_peers() {
        let entries = [mod_api::ModEntry::new("test", Box::new(MeasuredParticle))];
        mod_platform::__private::part_tests::install_parts_fixture(&entries).unwrap();
        crate::game::mods::resolve_particle_providers(&Default::default()).unwrap();
        let (mut hub, sunrise, vanilla) =
            test_hub("TypedParticle", vec!["minecraft:crit".into()], None);
        hub.guests.update_all(|guest| {
            if guest.conn == 1 {
                guest.brand = "sunrise".into();
            }
            guest.position = [1.0, 2.0, 3.0];
        });
        let typed = mod_api::__private::decode_particle_map(
            "test:measured",
            &mod_api::Compound::from([("measure".into(), mod_api::Tag::Int(257))]),
        )
        .unwrap()
        .unwrap();
        let options = ParticleOptions::typed(typed).unwrap();
        let at = [1.0, 2.0, 3.0];
        send_hub_level_particles(&hub, &options, at, [0.25, 0.5, 1.0], 2.0, 2, None);
        assert!(vanilla.try_recv().is_err());
        let mut packet = level_particles(&sunrise);
        let Some(ParticleData::Mod {
            name,
            options: bytes,
        }) = &packet.particle.data
        else {
            panic!("missing mod particle envelope")
        };
        assert_eq!(name, "test:measured");
        assert_eq!(bytes, &[1, 1, 0, 0]);
        assert!(decode_mod_particle(name, &bytes[..3]).is_none());
        assert!(decode_mod_particle(name, &[1, 1, 0, 0, 0]).is_none());
        let decoded = decode_mod_particle(name, bytes).unwrap();
        assert_eq!(decoded.typed_particle, options.typed_particle);
        let mut engine = sunrise_client::particles::ParticleEngine::new(
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        engine.created_requests = Some(Vec::new());
        let mut random = LegacyRandom::new(23);
        engine.handle_level_particles(&mut random, None, &packet, &decoded);
        let created = engine.created_requests.as_mut().unwrap();
        assert_eq!(created.len(), 2);
        for request in created.drain(..) {
            assert_eq!(request.0.typed_particle, decoded.typed_particle);
        }
        packet.count = 0;
        packet.dist = [0.1, -0.2, 0.7];
        packet.max_speed = [0.3, 0.4, 0.9];
        engine.handle_level_particles(&mut random, None, &packet, &decoded);
        let created = engine.created_requests.take().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].1, at);
        assert_eq!(
            created[0].2.map(f64::to_bits),
            [
                0x3f9e_b852_0000_0000,
                0xbfb4_7ae1_6000_0000,
                0x3fe4_28f5_c000_0000
            ]
        );

        send_hub_level_particles(
            &hub,
            &ParticleOptions::simple(ParticleType::from_name("crit").unwrap()),
            at,
            [0.0; 3],
            0.0,
            1,
            None,
        );
        let Out::Packet(vanilla) = vanilla.try_recv().unwrap() else {
            panic!("expected vanilla particle packet")
        };
        let Out::Packet(sunrise) = sunrise.try_recv().unwrap() else {
            panic!("expected vanilla particle packet")
        };
        assert_eq!(vanilla, sunrise);
        // Numeric particle ID with no mod envelope or appended stream body.
        let mut expected = vec![
            sunrise_protocol::ids::play::clientbound::LEVEL_PARTICLES as u8,
            0,
            0,
            0,
        ];
        for coordinate in at {
            expected.extend(coordinate.to_be_bytes());
        }
        expected.extend([0; 6 * 4]);
        expected.extend([1, 0]);
        assert_eq!(vanilla, expected);
    }

    /// `ShulkerBullet.hurtServer`: fifteen crit particles (spread 0.2, speed 0.0)
    /// are sent to tracking players within 32 blocks.
    #[test]
    fn hitting_shulker_bullet_sends_15_crit_particles_to_players() {
        let (hub, rx_near, rx_far) =
            test_hub("ShulkerBulletTest", vec!["minecraft:crit".into()], None);

        let at = [0.0, 64.0, 0.0];
        let options = ParticleOptions::simple(ParticleType::from_name("crit").unwrap());
        send_hub_level_particles(&hub, &options, at, [0.2, 0.2, 0.2], 0.0, 15, None);

        assert!(
            rx_far.try_recv().is_err(),
            "far guest should not receive particles"
        );

        let p = level_particles(&rx_near);
        assert_eq!(p.count, 15);
        assert_eq!(p.dist, [0.2, 0.2, 0.2]);
        assert_eq!(p.max_speed, [0.0; 3]);
        assert_eq!(p.pos, at);
        assert_eq!(p.particle.id, 0);
    }

    /// `VibrationSystem.Ticker.trySelectAndScheduleVibration`: the one packet
    /// is at the vibration's origin, count 1, no spread or speed, and carries
    /// the listener's `PositionSource` and the arrival time in ticks. The
    /// block listeners send a block position; the warden and allay send
    /// themselves (`EntityPositionSource`).
    #[test]
    fn vibration_particles_carry_their_listener_and_arrival_ticks() {
        // `Particle::read` reads a vibration's data from its pinned id, 58.
        let mut names = vec!["minecraft:empty".to_string(); 58];
        names.push("minecraft:vibration".to_string());
        let (hub, rx_near, rx_far) = test_hub("VibrationTest", names, None);

        let source = PositionSource::Block([3, 64, -7]);
        let at = [1.25, 64.0, 3.75];
        let options = ParticleOptions::vibration(source, [3.5, 64.5, -6.5], 14);
        send_hub_level_particles(&hub, &options, at, [0.0; 3], 0.0, 1, None);

        assert!(rx_far.try_recv().is_err(), "only players within 32 blocks");
        let p = level_particles(&rx_near);
        assert_eq!(p.count, 1, "one signal");
        assert_eq!(p.dist, [0.0; 3]);
        assert_eq!(p.max_speed, [0.0; 3]);
        assert_eq!(p.randomization, 0, "DEFAULT");
        assert_eq!(p.pos, at);
        assert_eq!(p.particle.id, 58);
        assert_eq!(
            p.particle.data,
            Some(ParticleData::Vibration {
                source: NetworkPositionSource::Block([3, 64, -7]),
                ticks: 14,
            })
        );

        // A listener entity travels as `EntityPositionSource`: its id and the
        // offset to its eyes.
        let source = PositionSource::Entity {
            id: 42,
            y_offset: 1.15,
        };
        // The render destination is not the transmitted identity of the listener.
        let options = ParticleOptions::vibration(source, [0.5, 65.0, 0.5], 7);
        send_hub_level_particles(&hub, &options, at, [0.0; 3], 0.0, 1, None);
        assert!(rx_far.try_recv().is_err(), "only players within 32 blocks");
        let p = level_particles(&rx_near);
        assert_eq!(p.count, 1);
        assert_eq!(p.dist, [0.0; 3]);
        assert_eq!(p.max_speed, [0.0; 3]);
        assert_eq!(p.randomization, 0);
        assert_eq!(p.pos, at);
        assert_eq!(p.particle.id, 58);
        assert_eq!(
            p.particle.data,
            Some(ParticleData::Vibration {
                source: NetworkPositionSource::Entity {
                    id: 42,
                    y_offset: 1.15,
                },
                ticks: 7,
            })
        );
        assert!(
            hub.particle(&ParticleOptions::simple(ParticleType::Vibration), None)
                .is_none()
        );
    }

    #[test]
    fn received_vibration_sources_preserve_block_coordinates_and_entity_offsets() {
        assert_eq!(
            particle_position_source(&NetworkPositionSource::Block([-12, 64, 300])),
            PositionSource::Block([-12, 64, 300]),
        );
        assert_eq!(
            particle_position_source(&NetworkPositionSource::Entity {
                id: 42,
                y_offset: 1.15,
            }),
            PositionSource::Entity {
                id: 42,
                y_offset: 1.15,
            },
        );
        // Identity conversion does not turn an invalid destination id into a
        // different entity; the destination lookup decides whether it exists.
        assert_eq!(
            particle_position_source(&NetworkPositionSource::Entity {
                id: -1,
                y_offset: -0.25,
            }),
            PositionSource::Entity {
                id: -1,
                y_offset: -0.25,
            },
        );
    }

    /// `ParticleTypes.STREAM_CODEC` for `ColorParticleOption` (`flash`, with
    /// `entity_effect` and `tinted_leaves`): the colour as its ARGB int.
    #[test]
    fn flash_particles_carry_their_colour_to_guests() {
        let (hub, rx_near, rx_far) = test_hub(
            "FlashTest",
            {
                // `Particle::read` reads a flash's colour from its pinned id, 52.
                let mut names = vec!["minecraft:empty".to_string(); 52];
                names.push("minecraft:flash".to_string());
                names
            },
            None,
        );

        let at = [0.0, 64.0, 0.0];
        let options = ParticleOptions::colored(ParticleType::Flash, [1.0, 0.0, 0.0, 1.0]);
        send_hub_level_particles(&hub, &options, at, [0.0; 3], 0.0, 1, None);

        assert!(rx_far.try_recv().is_err(), "only players within 32 blocks");
        let p = level_particles(&rx_near);
        assert_eq!(p.particle.id, 52);
        assert_eq!(
            p.particle.data,
            Some(ParticleData::Color(0xFFFF_0000u32 as i32))
        );
    }
}
