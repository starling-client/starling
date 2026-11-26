//! Block particles: the sprite and tint of a block's faces and particle material, honey slides,
//! and the burst a destroyed block leaves (`ClientLevel.addDestroyBlockEffect`).
use super::*;
use sunrise_client::{
    blocks::{MAP_COLORS, RenderShape},
    particles::{ParticleOptions, ParticleType},
};
use sunrise_core::geometry::Direction;

impl Game<'_> {
    /// `TerrainParticle` at `pos`: the model's particle sprite and tint source
    /// 0's `colorAsTerrainParticle` (white when the block has none).
    /// `HoneyBlock.showSlideParticles` (entity event 53): five honey block
    /// particles at the entity.
    pub(super) fn honey_slide_particles(&mut self, at: [f64; 3]) {
        self.honey_entity_particles(at, 53);
    }

    /// `HoneyBlock.showParticles`: the crumb count of entity event 53 or 54.
    pub(super) fn honey_entity_particles(&mut self, at: [f64; 3], event: i8) {
        let Some(count) = sunrise_client::landing::honey_particle_count(event) else {
            return;
        };
        let Some(honey) = self
            .pack
            .assets
            .registry
            .default_state("minecraft:honey_block")
        else {
            return;
        };
        let block = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        if let Some((sprite, tint)) = self.particle_sprite(honey, block) {
            for _ in 0..count {
                self.particles.add(
                    &ParticleOptions::block(ParticleType::Block, sprite.clone(), tint),
                    at,
                    [0.0; 3],
                );
            }
        }
    }

    /// `BlockStateModelSet.getParticleMaterial`: the sprite of the model's
    /// `particle` texture, else its first face (entity-rendered blocks may
    /// have a particle texture without faces, and faces without one).
    pub(super) fn block_particle_sprite(
        pack: &PackResources,
        state: BlockStateId,
    ) -> Option<String> {
        let catalog = &pack.render.model_catalog;
        let layer = catalog.particle(state).or_else(|| {
            let models = catalog
                .collect(state, sunrise_core::position::BlockPos::new(0, 0, 0))
                .ok()?;
            models
                .iter()
                .flat_map(|(_, m)| m.faces.iter())
                .next()
                .map(|f| f.layer)
        })?;
        pack.layer_names.get(&layer).cloned()
    }

    /// [`Game::block_particle_sprite`] as the particle atlas names it: the block atlas's
    /// `missingno` is packed there as `hud::MISSING_SPRITE`.
    fn particle_atlas_sprite(sprite: String) -> String {
        if sprite == "missingno" {
            sunrise_client::hud::MISSING_SPRITE.into()
        } else {
            sprite
        }
    }

    fn particle_shape(&self, pos: [i32; 3], state: BlockStateId) -> Vec<[f64; 6]> {
        let mut boxes = Vec::new();
        let _ = self.visit_state_shape(
            &self.world,
            &self.be_anims,
            state,
            pos,
            mod_api::BlockShapeKind::Outline,
            &mut |bounds| {
                boxes.push(bounds);
                std::ops::ControlFlow::Continue(())
            },
        );
        boxes
    }

    /// `ClientLevel.addDestroyBlockEffect`: decide using the destroyed state, not the block
    /// that now occupies `pos`, then add the default burst per outline box unless handled.
    pub(super) fn destroy_particles(&mut self, pos: [i32; 3], state: BlockStateId) {
        if self.settings.extras.hide_block_particles || self.res.facts().is_air(state) {
            return;
        }
        let spawn = self.res.facts().get(state).spawn_terrain_particles;
        if self.mod_add_destroy_effects(state, pos).unwrap_or(!spawn) {
            return;
        }
        let Some((sprite, tint)) = self.particle_sprite(state, pos) else {
            return;
        };
        let boxes = self.particle_shape(pos, state);
        sunrise_client::particles::destroy_positions(pos, &boxes, |p, v| {
            let particle = Particle::terrain(self.particles.random(), p, v, sprite.clone(), tint);
            self.particles.push(particle);
        });
    }

    /// `ClientLevel.addBreakingBlockEffects`' particle step, called after the sound with a
    /// non-air state and the hit face. Only visible blocks have hit particles.
    pub(super) fn crack_particle(&mut self, pos: [i32; 3], face: [i32; 3], state: BlockStateId) {
        let facts = self.res.facts().get(state);
        if self.settings.extras.hide_block_particles || facts.render_shape == RenderShape::Invisible
        {
            return;
        }
        let Some(face) = Direction::ALL.into_iter().find(|d| d.normal() == face) else {
            return;
        };
        let spawn = facts.spawn_terrain_particles;
        if !self.mod_add_hit_effects(state, pos, face).unwrap_or(!spawn) {
            self.add_breaking_particle(pos, state, face);
        }
    }

    /// Raw `ClientLevel.addBreakingParticles`: one crumb on the given face, with no sound or hook.
    pub(super) fn add_breaking_particle(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        face: Direction,
    ) -> bool {
        let Some((sprite, tint)) = self.particle_sprite(state, pos) else {
            return false;
        };
        let boxes = self.particle_shape(pos, state);
        let Some(p) = self.with_lent_level_random(|game| {
            sunrise_client::particles::breaking_position(
                game.particles.level_random(),
                pos,
                face.normal(),
                &boxes,
            )
        }) else {
            return false;
        };
        let mut particle = Particle::terrain(self.particles.random(), p, [0.0; 3], sprite, tint);
        particle.set_power(0.2);
        particle.scale(0.6);
        self.particles.push(particle);
        true
    }
    /// Resolve the three vanilla providers of `BlockParticleOption`. Render shape is
    /// a falling-dust gate, not a terrain or marker gate.
    pub(super) fn block_provider_options(
        &self,
        ty: ParticleType,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> Option<ParticleOptions> {
        self.pack.assets.registry.state(state)?;
        let facts = self.res.facts();
        let block = facts.get(state);
        let sprite = || {
            // BlockStateModelSet.getParticleMaterial uses the missing model when
            // the state has no model (e.g. air), not some other block's texture.
            Self::particle_atlas_sprite(
                Self::block_particle_sprite(&self.pack, state)
                    .unwrap_or_else(|| "missingno".into()),
            )
        };
        let options = match ty {
            ParticleType::Block | ParticleType::DustPillar | ParticleType::BlockCrumble => {
                if facts.is_air(state)
                    || &*block.name == "minecraft:moving_piston"
                    || !block.spawn_terrain_particles
                {
                    return None;
                }
                ParticleOptions::block(
                    ty,
                    sprite(),
                    self.terrain_particle_tint(state, pos).unwrap_or(0xFFFFFF),
                )
            }
            ParticleType::FallingDust => {
                if !facts.is_air(state) && block.render_shape == RenderShape::Invisible {
                    return None;
                }
                let map_color = || MAP_COLORS[usize::from(block.map_color)];
                let tint = match &*block.name {
                    "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil" => {
                        map_color()
                    }
                    name if name.starts_with("minecraft:")
                        && name.ends_with("_concrete_powder") =>
                    {
                        map_color()
                    }
                    name => sunrise_client::animate_tick::fixed_falling_dust_color(name)
                        .or_else(|| self.terrain_particle_tint(state, pos))
                        .unwrap_or_else(map_color),
                };
                // FallingDustParticle uses its own sprite set, never the block sprite.
                ParticleOptions::block(ty, String::new(), tint)
            }
            ParticleType::BlockMarker => ParticleOptions::block(ty, sprite(), 0xFFFFFF),
            _ => return None,
        };
        Some(options.with_source(&block.name))
    }

    pub(super) fn particle_sprite(
        &self,
        state: BlockStateId,
        pos: [i32; 3],
    ) -> Option<(String, u32)> {
        let sprite = Self::particle_atlas_sprite(Self::block_particle_sprite(&self.pack, state)?);
        let tint = self.terrain_particle_tint(state, pos).unwrap_or(0xFFFFFF);
        Some((sprite, tint))
    }

    fn terrain_particle_tint(&self, state: BlockStateId, pos: [i32; 3]) -> Option<u32> {
        let chunks = &self.world.chunks;
        let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
        let region: [[Option<&sunrise_client::chunk::Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| {
                chunks
                    .get(&(cx + dx as i32 - 1, cz + dz as i32 - 1))
                    .map(|c| &**c)
            })
        });
        self.pack.context.tables.terrain_particle_tint(
            state,
            pos,
            &region,
            world_biomes!(self),
            self.streaming.biome_zoom,
            self.streaming.biome_blend,
            self.streaming.nether_shading,
        )
    }
    /// `BlockTransformer.TransformParticle.send`: its level event.
    pub(super) fn transform_particle(&mut self, particle: &str, pos: [i32; 3]) {
        let source = self
            .lan_acting_uuid
            .map(sunrise_world::level::LevelEventPlayer);
        match particle {
            "scrape" => self.level_event_from(source, 3005, pos, 0),
            "wax_on" => self.level_event_from(source, 3003, pos, 0),
            "wax_off" => self.level_event_from(source, 3004, pos, 0),
            _ => {}
        }
    }
}

impl Game<'_> {
    /// The block's first up face (or any face) and its fixed tint, for block
    /// models drawn as entities (falling blocks, carried blocks).
    pub(super) fn falling_block_face_sprite(&self, state: BlockStateId) -> Option<(String, u32)> {
        let pack = &self.pack;
        let models = pack
            .render
            .model_catalog
            .collect(state, sunrise_core::position::BlockPos::new(0, 0, 0))
            .ok()?;
        let face = models
            .iter()
            .flat_map(|(_, m)| m.faces.iter())
            .find(|f| f.quad.facing == sunrise_assets::model::Direction::Up)
            .or_else(|| models.iter().flat_map(|(_, m)| m.faces.iter()).next())?;
        let name = pack.layer_names.get(&face.layer)?.clone();
        let tint = if face.quad.tint_index >= 0 {
            let n = &self.res.facts().get(state).name;
            if n.contains("leaves") || n.contains("vine") {
                0x48B518
            } else {
                0x91BD59
            }
        } else {
            0xFFFFFF
        };
        Some((name, tint))
    }
}
