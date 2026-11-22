//! Per-tick client visuals: the particle engine, `ClientLevel.animateTick`,
//! campfire smoke and rain splashes (`ClientLevel.tickWeatherEffects`).
use super::{Dim, Game, api_values::TryIntoApi, host::draw::GameCameraDrawContext};
use mod_api::{
    __private::{SelectedBlockScreenEffectRenderer, block_screen_effect_render},
    BlockScreenEffect,
};
use sunrise_client::assets::Properties;
use sunrise_client::camera::NearPlane;
use sunrise_client::fog::{FogData, FogInputs, FogType, settle_rain_fog, update_rain_fog};
use sunrise_client::{
    animate_tick::{AnimateContext, AnimateSampling, campfire_particles, marker_particle_target},
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    containers::BlockEntity,
    mobs::Mob,
    motion::{PlayerMotion, WATER},
    particles::{ParticleLevel, ParticleOptions, ParticleType, TrackingBody, TrackingTarget},
    world::World,
};
use sunrise_client::{
    hud_overlays,
    screen_effects::{
        CameraQuads, ScreenInputs, ScreenQuad, lightmap_brightness, nausea_has_effect,
    },
};
use sunrise_core::{
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use sunrise_world::dimension_type::{CardinalLight, Skybox};
use sunrise_world::{
    heightmap::HeightmapType, registry::BlockStateId, worldgen::terrain::TerrainBlockStates,
};

/// `ScreenEffectRenderer.submit`'s block overlays in the order it draws them, which
/// `BlockScreenEffects::custom` and `ScreenInputs::custom` follow.
const BLOCK_OVERLAYS: [BlockScreenEffect; 3] = [
    BlockScreenEffect::Block,
    BlockScreenEffect::Water,
    BlockScreenEffect::Fire,
];
/// The block overlays of `ScreenEffectRenderer` a frame shows.
struct BlockScreenEffects {
    /// The view-blocking block the camera is inside, drawn with its particle texture.
    block: Option<BlockStateId>,
    /// Draw the vanilla underwater overlay.
    water: bool,
    /// The player burns, seen in first person. Extraction cancellation does not change this.
    on_fire: bool,
    /// Whether a no-op or a custom renderer replaces the fire overlay.
    fire_suppressed: bool,
    /// The renderer for each overlay in [`BLOCK_OVERLAYS`], shown or canceled.
    custom: [Option<ExtractedBlockScreenEffect>; 3],
}

struct ExtractedBlockScreenEffect {
    state: mod_api::BlockState,
    pos: BlockPos,
    renderer: SelectedBlockScreenEffectRenderer,
}
pub(super) fn player_tracking_body(player: &PlayerMotion) -> TrackingBody {
    TrackingBody {
        feet: [player.position.x, player.position.y, player.position.z],
        width: f64::from(player.dimensions.width),
        height: f64::from(player.dimensions.height),
    }
}

pub(super) fn mob_tracking_body(mob: &Mob) -> TrackingBody {
    let (width, height, _) = mob.dims();
    TrackingBody {
        feet: [mob.position.x, mob.position.y, mob.position.z],
        width,
        height,
    }
}

/// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)` from the chunk's
/// heightmap: one above the highest block in `blocks_motion_in_heightmap` or
/// holding a fluid, however far above the camera it is.
pub(super) fn motion_blocking_height(
    world: &World,
    properties: &Properties,
    x: i32,
    z: i32,
) -> i32 {
    world.motion_blocking(x, z, |s| {
        HeightmapType::MotionBlocking.is_opaque(properties.heightmap_state(s))
    })
}
/// `ClientLevel.tickWeatherEffects`: the MOTION_BLOCKING height of the column
/// `x`, `z` when a rain splash may land on it for a camera in block row `y`,
/// that is above the world's bottom and within 10 blocks of the camera.
fn splash_height(world: &World, properties: &Properties, x: i32, y: i32, z: i32) -> Option<i32> {
    let top = motion_blocking_height(world, properties, x, z);
    (top > world.bottom_y() && (y - 10..=y + 10).contains(&top)).then_some(top)
}
/// The first block at or below `from` that blocks motion (leaves only when
/// `leaves`, as MOTION_BLOCKING_NO_LEAVES skips them) or holds a fluid, plus one.
fn motion_blocking_leaves(
    world: &World,
    facts: &BlockFacts,
    x: i32,
    z: i32,
    from: i32,
    leaves: bool,
) -> i32 {
    let mut y = from.min(world.top_y() - 1);
    while y > world.bottom_y() {
        let f = facts.get(world.block(x, y, z));
        // #minecraft:leaves block motion, but MOTION_BLOCKING_NO_LEAVES skips them
        // (their water still counts).
        let blocks = if f.name.ends_with("_leaves") {
            leaves
        } else {
            f.full_collision
        };
        if blocks || f.fluid != Fluid::None {
            return y + 1;
        }
        y -= 1;
    }
    world.bottom_y()
}
/// `Biome.getPrecipitationAt` is rain (not snow) at this position.
fn rains_at(world: &World, biomes: &Biomes, x: i32, y: i32, z: i32) -> bool {
    let Some(chunk) = world.chunks.get(&(x >> 4, z >> 4)) else {
        return false;
    };
    biomes.get(chunk.biome(x, y, z)).snows_at(x, y, z) == Some(false)
}

impl Game<'_> {
    /// Polytone GPU particles, before the frame's quads are built: any pending clear, the
    /// renderer's verdict on each shader and on the quads drawn here, then the heightmap for the readers that draw (a custom shader's `Sampler1`,
    /// and the surface rain drawn as quads dies under), every half second.
    pub(super) fn polytone_gpu_decide(&mut self, camera: glam::DVec3) {
        // A mod's clear asked since the last frame goes first: cleared records read nothing.
        self.apply_mod_particle_clear();
        let (world, properties) = (&self.world, &self.pack.assets.properties);
        let top = |x, z| motion_blocking_height(world, properties, x, z);
        let source = sunrise_client::particles::polytone_gpu::HeightmapSource {
            size: sunrise_client::gfx::POLYTONE_HEIGHTMAP_SIZE,
            min_y: world.bottom_y(),
            top: &top,
        };
        let renderer = self.renderer.as_mut().map(|r| r as _);
        self.particles
            .polytone_gpu_decide(renderer, camera, &source);
    }

    /// After the frame's quads (and any clear in their extraction): the records of the types
    /// the renderer draws with their own shader go to it.
    pub(super) fn polytone_gpu_particles(&mut self, camera: glam::DVec3, alpha: f64) {
        let world = &self.world;
        let renderer = self.renderer.as_mut().map(|r| r as _);
        self.particles
            .polytone_gpu_hand_over(renderer, camera, alpha as f32, &|b| {
                world.light(b[0], b[1], b[2])
            });
    }

    /// `GameRenderer`'s cardinal light: the level's type uses the Nether's diffuse lights.
    pub(super) fn nether_lighting(&self) -> bool {
        self.dimension_type().cardinal_light == CardinalLight::Nether
    }
    /// `SkyRenderer`'s overworld skybox: sun, moon, stars and clouds.
    pub(super) fn overworld_skybox(&self) -> bool {
        self.dimension_type().skybox == Skybox::Overworld
    }
    /// `SkyRenderer`'s End skybox and its flashes.
    pub(super) fn end_skybox(&self) -> bool {
        self.dimension_type().skybox == Skybox::End
    }
    pub(super) fn forward(&self) -> [f64; 3] {
        let (yaw, pitch) = (
            f64::from(self.yaw).to_radians(),
            f64::from(self.pitch).to_radians(),
        );
        [
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        ]
    }
    /// `Camera.getNearPlane` at the FOV option, as `getFluidInCamera` asks for it.
    pub(super) fn near_plane(&self) -> NearPlane {
        let [width, height] = self
            .renderer
            .as_ref()
            .map_or([1, 1], |r| [r.config.width, r.config.height]);
        self.view.near_plane(
            self.settings.fov as i32 as f32,
            width as f32,
            height.max(1) as f32,
        )
    }
    /// `FogRenderer.setupFog` for the frame's camera, and Sunrise's Fog option.
    pub(super) fn frame_fog(
        &self,
        camera: glam::DVec3,
        sky: &sunrise_client::environment::Sky,
        biome: u16,
        water_fog_color: [f32; 3],
    ) -> FogData {
        let forward = self.forward();
        let at = [camera.x, camera.y, camera.z];
        let fog_type = self.camera_fog_type(at);
        let (rain, thunder) = self.view_weather(self.clock.fraction() as f32);
        let title_fog = sunrise_client::fog::FogTables::default();
        let tables = if self.title.is_some() {
            &title_fog
        } else {
            self.res.fog()
        };
        let input = FogInputs {
            fog_type,
            sky,
            water_fog_color,
            biome,
            render_distance_chunks: self.terrain_distance(),
            distant_terrain: self.lod.is_some(),
            cloud_range_chunks: self.settings.cloud_range,
            camera_y: camera.y,
            min_y: self.level_min_y(),
            flat: self.level_flat(),
            forward: forward.map(|c| c as f32),
            rain,
            thunder,
            skybox: self.overworld_skybox(),
            blindness: self
                .potions
                .effects
                .get("blindness")
                .map(|effect| (effect.duration, effect.duration == -1)),
            darkness: self.darkness_fog(),
            darken_world: self
                .boss_overlay
                .darken_amount(self.clock.fraction() as f32),
            boss_world_fog: self.boss_overlay.should_create_world_fog(),
            spectator: self.spectator(),
            fire_resistance: self.potions.effects.get("fire_resistance").is_some()
                || self.pvp_clear_fluids().0,
            full_water_vision: self.pvp_clear_fluids().1,
            night_vision: self.fog_night_vision(),
            environment: self.pack.polytone_colors.env,
            fluid_fog: self.pack.polytone_colors.fluid_fog,
        };
        let mut fog = sunrise_client::fog::setup_fog(tables, &self.fog, &input);
        if !self.settings.sunrise.fog {
            sunrise_client::fog::clear_distance_fog(&mut fog, &input);
        }
        if self.title.is_none() {
            fog.color =
                self.native_fog_color_now(self.clock.fraction() as f32, at, fog_type, fog.color);
            fog = self.native_fog_now(self.clock.fraction() as f32, at, fog_type, fog);
        }
        fog
    }
    fn native_fog_color_now(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: FogType,
        color: [f32; 4],
    ) -> [f32; 4] {
        let mut rgb = [color[0], color[1], color[2]];
        self.native_compute_fog_color(partial, camera, fog_type, &mut rgb);
        if rgb.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
            [rgb[0], rgb[1], rgb[2], color[3]]
        } else {
            color
        }
    }
    fn native_fog_now(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: FogType,
        original: FogData,
    ) -> FogData {
        let mut fog = original;
        self.native_render_fog(partial, camera, fog_type, &mut fog);
        let distances = [
            fog.environmental_start,
            fog.environmental_end,
            fog.render_distance_start,
            fog.render_distance_end,
            fog.sky_end,
            fog.cloud_end,
        ];
        if fog
            .color
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && distances.iter().all(|v| v.is_finite())
            && fog.environmental_end > fog.environmental_start
            && fog.render_distance_end > fog.render_distance_start
            && fog.sky_end >= 0.0
            && fog.cloud_end >= 0.0
        {
            fog
        } else {
            original
        }
    }
    /// `FogRenderer.computeFogColor`'s night vision brightenFactor:
    /// `GameRenderer.nightVisionScale`, but not while Darkness is on.
    fn fog_night_vision(&self) -> f32 {
        let partial = self.clock.fraction() as f32;
        match self.potions.effects.get("night_vision") {
            Some(e) if self.potions.effects.get("darkness").is_none() => {
                // MobEffectInstance.endsWithin(200); -1 is infinite.
                if e.duration < 0 || e.duration > 200 {
                    1.0
                } else {
                    0.7 + ((e.duration as f32 - partial) * std::f32::consts::PI * 0.2).sin() * 0.3
                }
            }
            _ => 0.0,
        }
    }
    /// `ClientLevelData.isFlat`: on the server thread, the world-wide flag.
    fn level_flat(&self) -> bool {
        match self.remote.as_ref() {
            Some(r) => r.flat,
            None => self.world_flat,
        }
    }
    /// `Level.getMinY`: the current dimension type's minimum y.
    pub(super) fn level_min_y(&self) -> i32 {
        match self.remote.as_ref() {
            Some(r) => r.dimension.min_y,
            None => self.world.shape.min_y,
        }
    }
    /// `SkyRenderer.shouldRenderDarkDisc`: eye below the horizon and not under
    /// water. `ClientLevelData.getHorizonHeight`: the bottom of a flat world,
    /// otherwise y 63.
    pub(super) fn dark_disc(&self, camera: glam::DVec3) -> bool {
        let sunrise = if self.level_flat() {
            f64::from(self.level_min_y())
        } else {
            f64::from(self.pack.polytone_colors.env.horizon_height)
        };
        self.overworld_skybox()
            && camera.y < sunrise
            && self.camera_fog_type([camera.x, camera.y, camera.z]) != FogType::Water
    }
    /// `ScreenEffectRenderer` + `Hud` camera overlays for this frame.
    /// `scale` is the player's `LivingEntity.getScale`.
    /// `hand_projection` is the frame's `camera::hand_projection`, which mods draw under.
    pub(super) fn screen_quads(
        &self,
        partial: f32,
        size: [u32; 2],
        scale: f32,
        hand_projection: [f32; 16],
    ) -> Vec<ScreenQuad> {
        let Some(p) = self.player.as_ref() else {
            return Vec::new();
        };
        let eye = [
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ];
        let mut effects = self.block_screen_effects(p, eye, partial, size, scale);
        let custom = self.custom_screen_effects(&mut effects, partial, hand_projection);
        // ScreenEffectRenderer: the block's particle material.
        let block_overlay = effects
            .block
            .and_then(|state| Self::block_particle_sprite(&self.pack, state));
        let water = effects
            .water
            .then(|| (self.eye_brightness(eye), self.yaw, self.pitch));
        // Hud.extractCameraOverlays; the spyglass scope (and its easing) is
        // drawn on the HUD by draw_spyglass.
        let fx = &self.screen_fx;
        let overlays = hud_overlays::CameraOverlays {
            vignette: self.vignette_shown().then_some((
                fx.vignette_brightness,
                self.border.warning_strength(p.position.x, p.position.z),
            )),
            first_person: self.camera_type.is_first_person(),
            scoping: self.scoping(),
            game_time_delta: 0.0,
            equipment: [
                (self.inventory[self.selected].as_ref(), "mainhand"),
                (self.armor[3].as_ref(), "feet"),
                (self.armor[2].as_ref(), "legs"),
                (self.armor[1].as_ref(), "chest"),
                (self.armor[0].as_ref(), "head"),
                (self.offhand.as_ref(), "offhand"),
            ]
            .into_iter()
            .filter_map(|(stack, slot)| {
                let e = sunrise_client::item_sync::equippable(stack?, self.res.items())?;
                if e.slot != slot {
                    return None;
                }
                let overlay = e.camera_overlay?;
                Some(
                    overlay
                        .strip_prefix("minecraft:")
                        .map_or_else(|| overlay.clone(), str::to_owned),
                )
            })
            .collect(),
            ticks_frozen: p.ticks_frozen,
            percent_frozen: p.percent_frozen(),
            partial,
            portal: [fx.o_portal, fx.portal],
            nausea: self.nausea_blend(partial),
            screen_effect_scale: self.screen_effect_scale(),
        };
        let mut scope = self.scope_scale;
        let gui_overlays =
            hud_overlays::camera_overlays(size[0] as i32, size[1] as i32, &overlays, &mut scope);
        self.screen_fx.build(&ScreenInputs {
            sprites: &self.gui.atlas.sprites,
            flags: &self.gui.atlas.flags,
            aspect: size[0] as f32 / size[1].max(1) as f32,
            partial,
            block_overlay,
            water,
            on_fire: effects.on_fire && !effects.fire_suppressed,
            fire_lower: self.pvp_fire_lower(),
            gui_overlays: &gui_overlays,
            custom: [&custom[0], &custom[1], &custom[2]],
        })
    }
    /// Submit the selected renderers in vanilla order. Keep their quads separate so each
    /// goes immediately before its own vanilla overlay, if that overlay is still shown.
    fn custom_screen_effects(
        &self,
        effects: &mut BlockScreenEffects,
        partial: f32,
        hand_projection: [f32; 16],
    ) -> [Vec<ScreenQuad>; 3] {
        let mut drawn = <[Vec<ScreenQuad>; 3]>::default();
        for ((effect, custom), drawn) in BLOCK_OVERLAYS
            .into_iter()
            .zip(&effects.custom)
            .zip(&mut drawn)
        {
            let Some(custom) = custom else {
                continue;
            };
            let mut quads = CameraQuads::new(hand_projection);
            let Some(replace) = custom.renderer.submit(
                self,
                block_screen_effect_render(
                    effect,
                    custom.state,
                    custom.pos,
                    partial,
                    self.ui.hide_gui,
                    &mut GameCameraDrawContext::new(self, &mut quads),
                ),
            ) else {
                // Only a finished submission can affect the frame.
                continue;
            };
            if replace {
                match effect {
                    BlockScreenEffect::Block => effects.block = None,
                    BlockScreenEffect::Water => effects.water = false,
                    BlockScreenEffect::Fire => effects.fire_suppressed = true,
                }
            }
            *drawn = quads.take();
        }
        drawn
    }
    /// `LevelExtractor.extractPlayerState`: extract the player's block overlays and any
    /// renderers selected for them. Fire eligibility is separate from renderer selection.
    fn block_screen_effects(
        &self,
        p: &PlayerMotion,
        eye: [f64; 3],
        partial: f32,
        size: [u32; 2],
        scale: f32,
    ) -> BlockScreenEffects {
        // `LevelExtractor.getViewBlockingState`: 8 points around the eye, each
        // tested with `BlockState.isViewBlocking` against the near plane.
        // ScreenEffectRenderer: none of it for a spectator (noPhysics).
        let spectator = self.spectator();
        let modded = mod_api::__private::listened(mod_api::HookId::ExtractBlockScreenEffect);
        let player_partial_tick = self.clock.camera_fraction() as f32;
        let mut block = None;
        let mut custom = [None, None, None];
        if !spectator {
            // Frustum.getNearPlaneBounds, moved to the eye.
            let (near_min, near_max) = sunrise_client::camera::near_plane_bounds(
                self.fov_degrees(partial, eye),
                size[0] as f32,
                size[1] as f32,
                self.yaw,
                self.pitch,
                sunrise_client::camera::depth_far(
                    self.terrain_distance(),
                    self.settings.cloud_range,
                ),
            );
            let near_plane = sunrise_core::geometry::Aabb::new(
                sunrise_core::geometry::Vec3::new(
                    near_min[0] + eye[0],
                    near_min[1] + eye[1],
                    near_min[2] + eye[2],
                ),
                sunrise_core::geometry::Vec3::new(
                    near_max[0] + eye[0],
                    near_max[1] + eye[1],
                    near_max[2] + eye[2],
                ),
            );
            // The test points use float maths: `(i % 2 - 0.5F) * width * 0.8F`.
            // The height spread is `0.1F * getScale()`: a tiny player's eye is
            // closer to the ground than 0.05.
            let width = p.dimensions.width;
            for i in 0..8 {
                let t = [
                    p.position.x + f64::from(((i & 1) as f32 - 0.5) * width * 0.8),
                    p.position.y
                        + f64::from(p.dimensions.eye_height)
                        + f64::from((((i >> 1) & 1) as f32 - 0.5) * 0.1 * scale),
                    p.position.z + f64::from((((i >> 2) & 1) as f32 - 0.5) * width * 0.8),
                ];
                let pos = [
                    t[0].floor() as i32,
                    t[1].floor() as i32,
                    t[2].floor() as i32,
                ];
                let state = self.world.block(pos[0], pos[1], pos[2]);
                if self.res.facts().get(state).render_shape
                    != sunrise_client::blocks::RenderShape::Invisible
                    && self.state_view_blocking(state, pos, near_plane)
                {
                    block = Some(state);
                    if modded
                        && let Ok(state) = state.try_into_api()
                        && !self.screen_effect_kept(
                            BlockScreenEffect::Block,
                            state,
                            pos,
                            partial,
                            player_partial_tick,
                            &mut custom[0],
                        )
                    {
                        block = None;
                    }
                    break;
                }
            }
        }
        let first_person = self.camera_type.is_first_person() && !spectator;
        let water = first_person && {
            // LevelExtractor uses the player's tick-sampled eye state, not camera fog geometry.
            // Keep the existing mod override explicit, applied to that answer at the eye.
            let mut submersion = if p.is_eye_in_fluid(WATER) {
                FogType::Water
            } else {
                FogType::Atmospheric
            };
            self.native_camera_submersion(eye, &mut submersion);
            submersion == FogType::Water
                && (!modded
                    || self.screen_effect_kept(
                        BlockScreenEffect::Water,
                        self.pack.water_overlay_state,
                        eye.map(|c| c.floor() as i32),
                        partial,
                        player_partial_tick,
                        &mut custom[1],
                    ))
        };
        // Canceling fire acts as a no-op renderer only if no listener selected one.
        // A selected renderer can still prepend to the eligible vanilla overlay.
        let on_fire = first_person && p.is_on_fire();
        let fire_suppressed = on_fire
            && modded
            && self
                .pack
                .assets
                .registry
                .default_state("minecraft:fire")
                .and_then(|fire| fire.try_into_api().ok())
                .is_some_and(|fire| {
                    let feet = [p.position.x, p.position.y, p.position.z].map(|c| c.floor() as i32);
                    !self.screen_effect_kept(
                        BlockScreenEffect::Fire,
                        fire,
                        feet,
                        partial,
                        player_partial_tick,
                        &mut custom[2],
                    ) && custom[2].is_none()
                });
        BlockScreenEffects {
            block,
            water,
            on_fire,
            fire_suppressed,
            custom,
        }
    }
    /// Extract one overlay's renderer independently of whether vanilla's quads are kept.
    fn screen_effect_kept(
        &self,
        effect: BlockScreenEffect,
        state: mod_api::BlockState,
        pos: [i32; 3],
        partial_tick: f32,
        player_partial_tick: f32,
        custom: &mut Option<ExtractedBlockScreenEffect>,
    ) -> bool {
        let pos = BlockPos::new(pos[0], pos[1], pos[2]);
        let (mut canceled, mut renderer) = (false, None);
        self.native_extract_block_screen_effect(
            effect,
            state,
            pos,
            partial_tick,
            player_partial_tick,
            &mut canceled,
            &mut renderer,
        );
        *custom = renderer.map(|renderer| ExtractedBlockScreenEffect {
            state,
            pos,
            renderer,
        });
        !canceled
    }
    /// `Lightmap.getBrightness(getMaxLocalRawBrightness(eye))`.
    fn eye_brightness(&self, eye: [f64; 3]) -> f32 {
        let b = [
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        ];
        let packed = self.world.light(b[0], b[1], b[2]);
        let sky = i32::from(packed >> 4) - self.sky_darken();
        let raw = sky.max(i32::from(packed & 15)).clamp(0, 15) as u8;
        lightmap_brightness(if self.dim == Dim::Nether { 0.1 } else { 0.0 }, raw)
    }
    /// What `Minecraft` ticks outside the level's own tick: `GameRenderer.tick` runs the light
    /// flicker whenever the game is not paused, frozen or not, while `TextureManager.tick`
    /// needs the level running normally. `SUNRISE_STILL` keeps both with the world.
    pub(super) fn tick_frame_visuals(&mut self, running: bool) {
        self.hold_fullbright();
        if running || !self.clock.still() {
            self.fog.tick_flicker();
        }
        if running {
            self.texture_ticks += 1;
        }
    }
    /// What `AtmosphericFogEnvironment.updateRainFogState` reads at the eye: the rain, the sky
    /// light and whether the biome has precipitation.
    fn rain_fog_inputs(&self, eye: [f64; 3]) -> (f32, u8, bool) {
        let b = [
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        ];
        let rain = self.view_weather(1.0).0;
        let biome = self
            .world
            .chunks
            .get(&(b[0] >> 4, b[2] >> 4))
            .map_or(0, |c| c.biome(b[0], b[1], b[2]));
        let rains = world_biomes!(self).get(biome).precipitation;
        (rain, self.world.light(b[0], b[1], b[2]) >> 4, rains)
    }
    /// Under `SUNRISE_STILL` a frozen client's rain fog sits at its level, as the rain does: it
    /// eases only in `tick_particles`, which waits for the world.
    pub(super) fn settle_rain_fog(&mut self) {
        if let Some(e) = self.eye_position() {
            let (rain, sky_light, rains) = self.rain_fog_inputs(e);
            settle_rain_fog(&mut self.fog, rain, sky_light, rains);
        }
    }
    pub(super) fn remote_tracking_body(
        &mut self,
        id: i32,
    ) -> Option<(TrackingTarget, TrackingBody)> {
        if self.remote.as_ref().is_some_and(|r| r.entity_id == id) {
            let player = self.player.as_ref()?;
            return Some((TrackingTarget::LocalPlayer, player_tracking_body(player)));
        }
        let mob = self.mob_index.get(&self.mobs, id as u32)?;
        Some((TrackingTarget::Entity(id), mob_tracking_body(mob)))
    }

    /// `Hud.updateVignetteBrightness`, from the local player's eye: `Gui.tick` runs it on every tick
    /// the game isn't paused, frozen level or not.
    pub(in crate::game) fn tick_vignette(&mut self) {
        if let Some(e) = self.eye_position() {
            let brightness = self.eye_brightness(e);
            self.screen_fx.tick_vignette(brightness);
        }
    }

    pub(super) fn tick_particles(&mut self) {
        // A client mod's `clear_particles`, before this tick adds any.
        self.apply_mod_particle_clear();
        self.update_animation_inputs();
        let mut laps = sunrise_client::spans::laps();
        let eye = self.eye_position();
        if let Some(e) = eye {
            // A client of a server eases the vignette in `tick_hud`, frozen level or not.
            if self.remote.is_none() {
                let brightness = self.eye_brightness(e);
                self.screen_fx.tick_vignette(brightness);
            }
            let inside_portal = self.client_portal_effect();
            self.screen_fx.tick_portal(inside_portal);
            // LocalPlayer.aiStep: processPortalCooldown after the transition
            // effect (a local world's player counts down in handlePortal).
            if self.remote.is_some() && self.portals.player.cooldown > 0 {
                self.portals.player.cooldown -= 1;
            }
            let nausea = self.potions.effects.get("nausea").map(nausea_has_effect);
            self.screen_fx.tick_spinning(nausea);
            // LocalPlayer water vision and AtmosphericFogEnvironment rain fog.
            let in_water = self.camera_fog_type(e) == FogType::Water;
            self.fog.tick(in_water, self.spectator());
            let (rain, sky_light, rains) = self.rain_fog_inputs(e);
            update_rain_fog(&mut self.fog, rain, sky_light, rains, 1.0);
        }
        laps.lap("tp_fx");
        self.particles.particle_level = self.settings.particles;
        if let Some(e) = eye {
            self.particles.camera = e;
        }
        let (rain, _) = self.weather_levels(1.0);
        let marker = marker_particle_target(self.mode, self.stack_in(self.selected));
        // The day timeline: firefly bush sounds from 12600 to 23400.
        let firefly_bush_sounds = self.dimension_type().has_timeline("minecraft:day")
            && (12600..23401).contains(&self.world.day_time.rem_euclid(24000));
        // EnvironmentAttributes.CREAKING_ACTIVE: the same overworld night.
        let creaking_active = self.dimension_type().has_timeline("minecraft:day")
            && (12600..23401).contains(&self.world.day_time.rem_euclid(24000));
        // Level.isThundering: a thunder level above 0.9 where the sky is open.
        let thundering = self.can_have_weather() && self.world.weather.levels(1.0).1 > 0.9;
        let sky_darken = self.sky_darken();
        let ultrawarm = self.dimension_type().water_evaporates;
        // Whether the picks ask for the mod block parts' `animate_tick`.
        let parts = self.mod_animate_parts();
        let block = eye.map(|e| e.map(|v| v.floor() as i32));
        let mut sampling = block.map(AnimateSampling::new);
        // `ClientLevel.animateTick`, then the rain and the campfires. The sampling stops in each
        // pick whose block has a part, for the game to call it there (`mod_animate_tick`); the
        // context borrows the game, so it is made again for the rest of the tick.
        loop {
            let marker_options = |state| {
                // BlockMarker: the particle sprite, untinted.
                let sprite = Self::block_particle_sprite(&self.pack, state)?;
                Some(ParticleOptions::block(
                    ParticleType::BlockMarker,
                    sprite,
                    0xFFFFFF,
                ))
            };
            let world = &self.world;
            let facts = self.res.facts();
            let biomes = world_biomes!(self);
            let properties = &self.pack.assets.properties;
            let raining = rain > 0.2;
            let raining_at = |p: [i32; 3]| {
                raining
                    && motion_blocking_height(world, properties, p[0], p[2]) <= p[1]
                    && rains_at(world, biomes, p[0], p[1], p[2])
            };
            let height_no_leaves =
                |p: [i32; 3]| motion_blocking_leaves(world, facts, p[0], p[2], i32::MAX, false);
            let tables = &self.pack.context.tables;
            let model_offset = |state, x, z| tables.offset(state, x, z);
            let gateway_particles = |state, p| {
                self.res
                    .block_entity_render()
                    .gateway_faces(world, state, p)
                    .len()
            };
            let ctx = AnimateContext {
                enchant_power: &self.ambience.enchant_power,
                world,
                facts,
                collision: self.collision,
                biomes,
                registry: self.pack.assets.registry,
                raining_at: &raining_at,
                height_no_leaves: &height_no_leaves,
                firefly_bush_sounds,
                creaking_active,
                sky_darken,
                marker: marker.map(|name| (name, &marker_options as &dyn Fn(_) -> _)),
                model_offset: &model_offset,
                gateway_particles: &gateway_particles,
                thundering,
                ultrawarm,
                block_parts: parts
                    .then_some(&Self::animate_part_listens as &dyn Fn(BlockStateId) -> bool),
            };
            if let (Some(block), Some(sampling)) = (block, sampling.as_mut()) {
                if !sampling.started() {
                    laps.lap("tp_ctx");
                    self.particles
                        .set_polytone(self.pack.polytone_particles.clone());
                    if let Some(registry) = &self.pack.polytone_particles {
                        registry.set_biomes(biomes);
                    }
                }
                if let Some((state, pos)) =
                    sampling.run(self.res.animate(), &mut self.particles, &ctx)
                {
                    // `Block.animateTick` of the picked block's parts, inside its pick: after the
                    // block's own effects, before its fluid's and the next pick.
                    self.mod_animate_tick(state, pos);
                    continue;
                }
                laps.lap("tp_animate");
                // The Sunrise Weather option off means no rain visuals or sounds at all.
                if rain > 0.0 && self.settings.sunrise.weather {
                    // ClientLevel.tickWeatherEffects: rain splashes within the weather
                    // radius, and the rain sound.
                    let mut random =
                        LegacyRandom::new(self.world.game_time.wrapping_mul(312_987_231));
                    let radius = self.settings.weather_radius;
                    let diameter = 2 * radius + 1;
                    let count = (0.225 * (diameter * diameter) as f32 * rain * rain) as i32
                        / if self.settings.particles == 1 { 2 } else { 1 };
                    let mut rain_position = None;
                    for _ in 0..count {
                        let x = block[0] + random.next_int_bound(diameter).unwrap_or(0) - radius;
                        let z = block[2] + random.next_int_bound(diameter).unwrap_or(0) - radius;
                        let Some(top) = splash_height(world, properties, x, block[1], z) else {
                            continue;
                        };
                        if !rains_at(world, biomes, x, top, z) {
                            continue;
                        }
                        let below = top - 1;
                        rain_position = Some([x, below, z]);
                        // The Minimal option stops at the first spot that rains (the
                        // rain sound still plays there).
                        if self.settings.particles >= 2 {
                            break;
                        }
                        let (bx, bz) = (random.next_double(), random.next_double());
                        let state = world.block(x, below, z);
                        let f = facts.get(state);
                        let block_top = self
                            .collision
                            .cached_shapes()
                            .cached_collision_top(state, [x, below, z], bx, bz)
                            .ok()
                            .flatten()
                            .unwrap_or(0.0);
                        let fluid_top = if f.fluid == Fluid::None {
                            0.0
                        } else {
                            f64::from(f.fluid_level) / 9.0
                        };
                        let smoke = f.fluid == Fluid::Lava
                            || &*f.name == "minecraft:magma_block"
                            || (f.name.ends_with("campfire")
                                && self.res.animate().is_lit_campfire(state.0));
                        let ty = if smoke {
                            ParticleType::Smoke
                        } else {
                            ParticleType::Rain
                        };
                        let p = [
                            f64::from(x) + bx,
                            f64::from(below) + block_top.max(fluid_top),
                            f64::from(z) + bz,
                        ];
                        self.particles
                            .add(&ParticleOptions::simple(ty), p, [0.0; 3]);
                    }
                    // The last rain column of the tick makes the sound, more often the
                    // longer it has been quiet; indoors it is the quieter one.
                    if let Some(at) = rain_position {
                        let waited = self.rain_sound_time;
                        self.rain_sound_time += 1;
                        if random.next_int_bound(3).unwrap_or(0) < waited {
                            self.rain_sound_time = 0;
                            let roof =
                                motion_blocking_height(world, properties, block[0], block[2]);
                            let (name, volume, pitch) = if at[1] > block[1] + 1 && roof > block[1] {
                                ("minecraft:weather.rain.above", 0.1, 0.5)
                            } else {
                                ("minecraft:weather.rain", 0.2, 1.0)
                            };
                            let p = at.map(|c| f64::from(c) + 0.5);
                            // playLocalSound: guests make their own rain, so a host
                            // must not capture it for them.
                            self.audio.play_unshared(name, Some(p), volume, pitch);
                        }
                    }
                }
            }
            if let Some(r) = &self.renderer {
                campfire_particles(
                    self.res.animate(),
                    &mut self.particles,
                    &ctx,
                    r.block_entities(),
                    &|pos| match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
                        Some(BlockEntity::Campfire(c)) => c.items.each_ref().map(Option::is_some),
                        _ => [false; 4],
                    },
                );
            }
            laps.lap("tp_rain_campfire");
            break;
        }
        let world = &self.world;
        let facts = self.res.facts();
        // SpellParticle.isCloseToScopingPlayer: the eye of a local player looking
        // through a spyglass in first person.
        let scope_eye = self
            .player
            .as_ref()
            .filter(|_| self.camera_type.is_first_person() && self.scoping())
            .map(|p| {
                [
                    p.position.x,
                    p.position.y + f64::from(p.dimensions.eye_height),
                    p.position.z,
                ]
            });
        self.particles.set_scope_eye(scope_eye);
        let level = ParticleLevel {
            world,
            facts,
            collision: self.collision,
            player: self
                .player
                .as_ref()
                .map(|p| ([p.position.x, p.position.y, p.position.z], p.velocity.y)),
            cached_chunk: std::cell::Cell::new(None),
        };
        laps.lap("tp_level");
        // Resource packs' Polytone entity emitters on the player (#11486): fog, dust and
        // weather around them; nothing without such a pack.
        if let Some(p) = &self.player {
            let feet = [p.position.x, p.position.y, p.position.z];
            self.particles
                .polytone_entity(&level, "minecraft:player", feet);
        }
        let player = self.player.as_ref();
        let mobs = &self.mobs;
        let mob_index = &mut self.mob_index;
        let lan = self.lan.as_ref();
        let dimension = self.dim.id();
        self.particles.tick_tracking(&level, |target| match target {
            TrackingTarget::LocalPlayer => player.map(player_tracking_body),
            TrackingTarget::Entity(id) => {
                if let Some(guest) = lan.and_then(|host| {
                    host.hub
                        .guests
                        .iter()
                        .find(|guest| guest.entity_id == id && guest.dimension == dimension)
                }) {
                    let dimensions = guest.dimensions();
                    return Some(TrackingBody {
                        feet: guest.position,
                        width: f64::from(dimensions.width),
                        height: f64::from(dimensions.height),
                    });
                }
                mob_index.get(mobs, id as u32).map(mob_tracking_body)
            }
        });
        laps.lap("tp_engine");
        self.tick_pickups();
        laps.lap("tp_pickups");
        let camera = self.particles.camera;
        for (name, at, volume, pitch, delayed) in std::mem::take(&mut self.particles.sounds) {
            if delayed {
                self.audio
                    .play_distance_delayed(name, at, volume, pitch, camera);
            } else {
                self.audio.play(name, Some(at), volume, pitch);
            }
        }
    }

    /// The ticks the frame shows (animated textures, the HUD, a shader pack's clock): the
    /// client's, or under `SUNRISE_STILL` (`FixedStep::still`) the world's game time, which
    /// moves only while the world runs.
    pub(super) fn visual_ticks(&self) -> u64 {
        if self.clock.still() {
            self.world.game_time.max(0) as u64
        } else {
            self.ticks
        }
    }

    /// The clock of the animated textures: `Minecraft.runTick` ticks the `TextureManager`
    /// only while the level runs normally. Under `SUNRISE_STILL` it is the world's ticks.
    pub(super) fn texture_clock(&self) -> u64 {
        if self.clock.still() {
            self.visual_ticks()
        } else {
            self.texture_ticks
        }
    }

    /// `Util.getMillis` for what the frame shows (the world border's scroll, the hearts' blink,
    /// the glint's under the switch): under `SUNRISE_STILL`, the world's ticks in milliseconds.
    pub(super) fn visual_millis(&self) -> i64 {
        if self.clock.still() {
            self.visual_ticks() as i64 * 50
        } else {
            self.started.elapsed().as_millis() as i64
        }
    }
}
impl Game<'_> {
    /// What the packs' Polytone animation modes read this tick (wiki "Variant Textures"): the
    /// time of day, the weather, and for expressions the packs' Polytone frame, or the time and
    /// weather alone when no Polytone pack makes one.
    fn update_animation_inputs(&mut self) {
        let (rain, thunder) = self.view_weather(0.0);
        let day_time = self.view_day_time();
        let frame = self
            .pack
            .polytone_particles
            .as_ref()
            .and_then(|p| p.frame.read().ok().map(|f| f.clone()))
            .unwrap_or_else(|| sunrise_client::environment::Frame {
                rain,
                thunder,
                game_time: self.world.game_time,
                day_time,
                ..Default::default()
            });
        // `screen_time` counts while the same screen stays open.
        let screen = (self.screen != super::Screen::Playing).then(|| {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::mem::discriminant(&self.screen).hash(&mut hasher);
            hasher.finish()
        });
        if let Some(renderer) = &mut self.renderer {
            renderer.set_animation_inputs(day_time, [rain, thunder], Some(&frame), screen);
        }
    }
}

/// The packs' expression uniforms on core shaders (`shader_modifiers`, wiki "Shaders").
impl Game<'_> {
    /// Whether an installed pack core shader reads expression uniforms this frame. Fullbright
    /// excludes the lightmap only, and a frame where no world runs the text
    /// ([`Self::world_runs`]). The frame's expression context is made only when needed.
    pub(in crate::game) fn reads_pack_expressions(&mut self) -> bool {
        let world = self.world_runs();
        (self.renderer.as_mut()).is_some_and(|r| r.reads_pack_expressions(world))
    }

    /// Whether an installed pack core shader reads Polytone's `PolyGlobals` this frame (the
    /// text's only where a world runs).
    pub(in crate::game) fn reads_poly_globals(&mut self) -> bool {
        let world = self.world_runs();
        (self.renderer.as_mut()).is_some_and(|r| r.reads_poly_globals(world))
    }

    /// The frame's `PolyGlobals` for the pack core shaders that read them; none leaves them 0.
    pub(in crate::game) fn give_poly_globals(
        &mut self,
        globals: Option<&sunrise_client::polytone_post::PolyGlobals>,
    ) {
        let world = self.world_runs();
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_poly_globals(globals.map(|g| g.bytes()).as_ref(), world);
        }
    }

    /// Evaluate the installed pack pipelines' expression uniforms from `frame`, skipping the
    /// lightmap under fullbright, and the text's where no world runs
    /// ([`Self::world_runs`]). Values are uploaded when each pipeline prepares its draw.
    pub(in crate::game) fn evaluate_pack_expressions(
        &mut self,
        frame: Option<&sunrise_client::environment::Frame>,
    ) {
        let world = self.world_runs();
        if let (Some(frame), Some(renderer)) = (frame, self.renderer.as_mut()) {
            renderer.evaluate_pack_expressions(frame, world);
        }
    }

    /// Whether the text's Polytone blocks are given this frame: only where a world runs. On a
    /// title frame (the title screen and its menus) the renderer writes them as 0, as it does on
    /// the loading, file fixer and saving screens' frames.
    fn world_runs(&self) -> bool {
        !self.shows_panorama()
    }
}

#[cfg(test)]
mod tests {
    use super::{Properties, World, splash_height};
    use std::sync::Arc;
    use sunrise_client::chunk::Chunk;
    use sunrise_world::registry::BlockStateId;

    /// #3810: under a stone roof more than 64 blocks above the camera, as in a
    /// deep excavation, MOTION_BLOCKING is the roof, so no rain splash lands on
    /// the floor; with the roof gone, it does.
    #[test]
    fn no_rain_splash_under_a_high_roof() {
        // State 0 is air, 1 is stone.
        let properties =
            Properties::for_tests(&[(true, false, false, false), (false, false, true, true)]);
        let stone = BlockStateId(1);
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        for x in 0..16 {
            for z in 0..16 {
                world.set(x, 0, z, stone).unwrap();
            }
        }
        // The camera stands on the floor, its block at y 1.
        assert_eq!(splash_height(&world, &properties, 3, 1, 3), Some(1));
        for roof in [70, 80, 200] {
            world.set(3, roof, 3, stone).unwrap();
            assert_eq!(
                splash_height(&world, &properties, 3, 1, 3),
                None,
                "roof at {roof}"
            );
            world.set(3, roof, 3, BlockStateId(0)).unwrap();
        }
        assert_eq!(splash_height(&world, &properties, 3, 1, 3), Some(1));
        // A roof within 10 blocks is where the splash lands.
        world.set(3, 8, 3, stone).unwrap();
        assert_eq!(splash_height(&world, &properties, 3, 1, 3), Some(9));
    }

    /// `extract_block_screen_effect`: a mod that cancels the overlay of stone at one position
    /// leaves the frame's block overlays as they are with no block there, the fire overlay
    /// included; dirt in the same place keeps its overlay.
    #[test]
    #[ignore = "daily"]
    fn a_mod_cancels_the_screen_effect_of_one_block() {
        use mod_api::{
            BlockScreenEffect, ClientHost, ExtractBlockScreenEffectEvent, Flow, Hooks, ModEntry,
            Mods,
        };
        use sunrise_core::{geometry::Vec3, position::BlockPos};
        static SEEN: std::sync::Mutex<Vec<(BlockScreenEffect, BlockStateId, BlockPos)>> =
            std::sync::Mutex::new(Vec::new());
        struct Mod(BlockStateId);
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn extract_block_screen_effect(
                &self,
                _: &dyn ClientHost,
                event: &mut ExtractBlockScreenEffectEvent<'_>,
            ) -> Flow {
                SEEN.lock().unwrap().push((
                    event.effect,
                    crate::game::api_values::TryIntoEngine::try_into_engine(&event.state, &())
                        .unwrap(),
                    event.pos,
                ));
                if event.effect == BlockScreenEffect::Block
                    && crate::game::api_values::TryIntoEngine::try_into_engine(&event.state, &())
                        .unwrap()
                        == self.0
                    && event.pos == BlockPos::new(0, 65, 0)
                {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        super::super::native_entity_refs::tests::with_game(|game| {
            let registry = &game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let (air, stone, dirt, fire) = (
                state("minecraft:air"),
                state("minecraft:stone"),
                state("minecraft:dirt"),
                state("minecraft:fire"),
            );
            game.world.chunks.insert(
                (0, 0),
                Arc::new(Chunk::with_layout(0, 0, game.world.layout())),
            );
            // The eye is at y 65.62, so all eight test points are in block (0, 65, 0).
            let mut player =
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.position = Vec3::new(0.5, 64.0, 0.5);
            player.fire_ticks = 100;
            game.player = Some(player);
            let effects = |game: &mut super::Game<'_>, block| {
                game.world.set(0, 65, 0, block).unwrap();
                let p = game.player.as_ref().unwrap();
                let eye = [0.5, 64.0 + f64::from(p.dimensions.eye_height), 0.5];
                let effects = game.block_screen_effects(p, eye, 0.0, [854, 480], 1.0);
                assert!(effects.custom.iter().all(Option::is_none));
                (effects.block, effects.water, effects.on_fire)
            };
            let open = effects(game, air);
            let in_dirt = effects(game, dirt);
            assert_eq!(effects(game, stone), (Some(stone), false, true));
            let mods: Mods = vec![ModEntry::new("screen-effect", Box::new(Mod(stone)))].into();
            mod_platform::testing::with_mods(mods, || {
                assert_eq!(effects(game, stone), open);
                assert_eq!(effects(game, dirt), in_dirt);
            });
            assert_eq!(in_dirt, (Some(dirt), false, true));
            assert_eq!(open, (None, false, true));
            let seen = SEEN.lock().unwrap();
            let feet = BlockPos::new(0, 64, 0);
            assert_eq!(
                seen[..],
                [
                    (BlockScreenEffect::Block, stone, BlockPos::new(0, 65, 0)),
                    (BlockScreenEffect::Fire, fire, feet),
                    (BlockScreenEffect::Block, dirt, BlockPos::new(0, 65, 0)),
                    (BlockScreenEffect::Fire, fire, feet),
                ]
            );
        });
    }
}
