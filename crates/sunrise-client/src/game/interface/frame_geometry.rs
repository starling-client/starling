//! Geometry the frame builds on the CPU each time it draws: the breaking cracks (`cracks`), the sun,
//! moon and stars (`celestial_geometry`), the first-person hand and arm (`hand_geometry`,
//! `first_person_arm`) and the rain and snow around the camera (`weather_geometry`).
use super::*;
use sunrise_client::{gfx::CelestialBody, mobs::ModelNormals};

impl Game<'_> {
    /// Stored local and remote display positions, keeping the furthest stage on each cell.
    pub(super) fn crack_stages(&self) -> Vec<([i32; 3], u32)> {
        let mut stages: Vec<([i32; 3], u32)> = Vec::new();
        let mut add = |pos: [i32; 3], stage: u32| match stages.iter_mut().find(|c| c.0 == pos) {
            Some(c) => c.1 = c.1.max(stage),
            None => stages.push((pos, stage)),
        };
        if let Some(m) = &self.mining
            && let Some(stage) = m.stage()
        {
            add(m.pos, u32::from(stage));
            if let Some(crack) = &m.crack {
                for &pos in &crack.positions {
                    add(pos, u32::from(stage));
                }
            }
        }
        let now = self.world.game_time;
        for crack in self.destroying.values() {
            if now.saturating_sub(crack.updated_at) <= 400 {
                add(crack.pos, u32::from(crack.stage));
                for &pos in &crack.positions {
                    add(pos, u32::from(crack.stage));
                }
            }
        }
        stages
    }

    /// `renderBlockDestroyAnimation`: eligible cells within 32 blocks of the camera,
    /// with each stage sprite's atlas origin. No hook is asked during extraction.
    pub(super) fn cracks(&self, camera: glam::DVec3) -> Vec<([i32; 3], [f32; 2])> {
        let names = crack_stage_names(self.settings.improved_transparency);
        self.crack_stages()
            .into_iter()
            .filter(|(pos, _)| {
                let d = glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                    + 0.5
                    - camera;
                d.length_squared() <= 1024.0
            })
            .filter(|(pos, _)| {
                let state = self.world.block(pos[0], pos[1], pos[2]);
                self.res.facts().get(state).destroy_overlay
            })
            .filter_map(|(pos, stage)| {
                let name = names.get(stage as usize)?;
                let r = self.gui.atlas.sprites.get(*name)?;
                Some((pos, [r.u0, r.v0]))
            })
            .collect()
    }
    /// SkyRenderer.renderSunMoonAndStars: bodies on a frame rotated Y -90 then X by angle.
    /// Returns the sun and moon quads with what each is (`celestial_bodies`), and the stars'
    /// frame and brightness (the renderer keeps their geometry, built once). The bodies fade
    /// with the rain (`rainBrightness = 1 - rainLevel`, their colour's alpha).
    pub(super) fn celestial_geometry(
        &self,
        sky: &sunrise_client::environment::Sky,
        rain: f32,
    ) -> (Vec<Vertex>, Vec<CelestialBody>, Option<(glam::Mat4, f32)>) {
        let body_color =
            0x00FF_FFFF | (((1.0 - rain).clamp(0.0, 1.0) * 255.0).round() as u32) << 24;
        let phases = [
            "full_moon",
            "waning_gibbous",
            "third_quarter",
            "waning_crescent",
            "new_moon",
            "waxing_crescent",
            "first_quarter",
            "waxing_gibbous",
        ];
        let phase = phases[(self.view_day_time() / 24000).rem_euclid(8) as usize];
        let moon = format!("environment/celestial/moon/{phase}");
        let (out, bodies) = celestial_bodies(&self.gui.atlas.sprites, sky, &moon, body_color);
        // SkyRenderer.renderStars: the static star buffer on the same frame.
        let stars = (sky.star_brightness > 0.0)
            .then(|| (celestial_frame(sky.star_angle), sky.star_brightness));
        (out, bodies, stars)
    }
    /// `renderPlayerHand`: ask `RenderArmEvent` before submitting the default arm.
    pub(super) fn first_person_arm(
        &self,
        left: bool,
        m: glam::Mat4,
        light: u32,
        partial: f32,
        out: &mut Vec<Vertex>,
    ) {
        if mod_api::__private::listened(mod_api::HookId::RenderArm) {
            let eye = self.eye_position().unwrap_or_default();
            let mut draw = BlockEntityDraw::posed(
                partial,
                self.world.game_time,
                light,
                eye.map(|c| c.floor() as i32),
                m,
                sunrise_client::block_entity_render::diffuse_lights(self.dim == Dim::Nether),
                std::mem::take(out),
            )
            .animated(self.animation_time());
            let mut canceled = false;
            self.native_render_arm(
                self.own_uuid(),
                self.own_entity_id(),
                left,
                &mut draw,
                &mut canceled,
            );
            *out = draw.into_vertices();
            if canceled {
                return;
            }
        }
        self.submit_first_person_arm(left, (m, ModelNormals::new(m)), light, out);
    }

    /// `AvatarRenderer.renderHand`: submit the posed avatar arm without posting another hook.
    pub(in crate::game) fn submit_first_person_arm(
        &self,
        left: bool,
        base: (glam::Mat4, ModelNormals),
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let Some(avatar) = self.player_model.as_ref() else {
            return;
        };
        let Some(arm) = self.res.player_arm(avatar.slim, left).as_ref() else {
            return;
        };
        // Selection resolves a texture and model as a pair before submission.
        let sprites = &self.gui.atlas.sprites;
        let Some(r) = avatar.skin.as_deref().and_then(|skin| sprites.get(skin)) else {
            return;
        };
        let (sleeve, bit, z_rot) = if left {
            ("left_sleeve", 2, -0.1)
        } else {
            ("right_sleeve", 3, 0.1)
        };
        let hide_sleeve = avatar.extra.hidden_model_parts & (1 << bit) != 0;
        let pose = |p: &sunrise_client::mobs::Part| {
            if hide_sleeve && p.name == sleeve {
                return None;
            }
            let mut pose = p.pose;
            pose[3] = 0.0;
            pose[4] = 0.0;
            pose[5] = if std::ptr::eq(p, arm) { z_rot } else { 0.0 };
            Some(pose)
        };
        sunrise_client::mobs::render_model_filtered(
            arm,
            base,
            &pose,
            [r.u0, r.v0, r.u1, r.v1],
            light,
            0,
            out,
        );
    }
    /// FirstPersonHandsAndItemsRenderer: the right arm or the held items of
    /// both hands in view space, converted to camera-relative world space,
    /// and where the arm and each item start, with the `currentRenderedItemId`
    /// a shader pack reads for them.
    pub(super) fn hand_geometry(&mut self, alpha: f64) -> (Vec<Vertex>, Vec<(u32, i32)>) {
        let mut out = Vec::new();
        let mut runs = Vec::new();
        // F1 hides both; a native HUD mode may keep the hands visible. The
        // replay player's hands remain visible in first person; freecam has
        // no player-view hand.
        if !fov::studio_draws_hand(
            self.studio.flying(),
            self.ui.hands_drawn(
                self.spectator(),
                self.sleeping.is_some(),
                self.camera_type.is_first_person(),
                self.player
                    .as_ref()
                    .is_some_and(|p| !p.health.dead_or_dying()),
                self.screen == Screen::Dead,
            ),
        ) {
            return (out, runs);
        }
        let a = alpha as f32;
        let interpolated_pitch = self.player_model.as_ref().map_or(self.pitch, |model| {
            model.previous_head_pitch + a * (self.pitch - model.previous_head_pitch)
        });
        // FirstPersonHandsAndItemsRenderer: the swing goes to the hand that
        // swings; each hand has its inverse arm height (the swap scale is 1
        // for every item but spears).
        let swing = self.swing.animation(a);
        let attacks = Hand::BOTH.map(|h| {
            if self.swing.attack_hand() == h {
                swing
            } else {
                0.0
            }
        });
        let heights = self.heights.interpolated(a);
        // The hand sway: both hands turn by a tenth of the bobs' lag.
        let to_world = self.sway.to_world(a, &self.view);
        let eye = self.eye_position().unwrap_or_default();
        let light = self.world.light(
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        );
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let shown = self.hands_shown();
        let used_hand = self.used_hand();
        let spinning = self
            .player
            .as_ref()
            .is_some_and(|p| p.auto_spin_attack_ticks > 0);
        let items = self.visible_hand_items();
        let off_hand_empty = items[1].is_none();
        // Combat feel Legacy: a sword is held up while use is held, as 1.8.9's block looked.
        // Only the picture changes: the clicks and what they send stay 26.3's.
        let legacy_block = self.pvp_legacy_look()
            && self.mapped_key_down("key.use")
            && self.screen == Screen::Playing
            && items[0]
                .as_ref()
                .is_some_and(|s| s.item.ends_with("_sword"))
            && match used_hand {
                None => true,
                Some(Hand::Off) => items[1]
                    .as_ref()
                    .is_some_and(|s| &*s.item == "minecraft:shield"),
                Some(Hand::Main) => false,
            };
        for (i, (hand, item)) in Hand::BOTH.into_iter().zip(items).enumerate() {
            if !shown[i] {
                continue;
            }
            let scale = item.as_ref().map_or(1.0, |stack| {
                self.pack
                    .item_renderer
                    .models
                    .swap_animation_scale(&sunrise_client::item_render::stack_view(stack))
            });
            let attack = attacks[i];
            let inverse_height =
                sunrise_client::first_person::equip_displacement(heights[i], scale);
            let snapshot = if mod_api::__private::listened(mod_api::HookId::RenderHand) {
                item.as_ref()
                    .filter(|stack| stack.count > 0)
                    .try_into_api()
                    .ok()
            } else {
                None
            };
            let mut render = mod_api::HandRender {
                player: self.own_uuid(),
                player_entity_id: self.own_entity_id(),
                hand,
                stack: snapshot.as_ref().and_then(Option::as_ref),
                partial: a,
                interpolated_pitch,
                swing: attack,
                equip: inverse_height,
                // LightCoordsUtil.pack(block, sky), as RenderHandEvent
                // gives it, not the vertex's encoding.
                packed_light: u32::from(light & 15) << 4 | u32::from(light >> 4) << 20,
                translation: [0.0; 3],
                rotation: [0.0; 3],
                scale: 1.0,
            };
            // RenderHandEvent: the mods pose the hand, draw in it or leave vanilla's out.
            if snapshot.is_some()
                && self.render_hand_cancelled(&mut render, to_world, packed, &mut out, &mut runs)
            {
                continue;
            }
            // No mod moved the hand (always so without one): no matrices to build.
            let [x, y, z] = render.translation;
            let [pitch, yaw, roll] = render.rotation;
            let pose = [x, y, z, pitch, yaw, roll, render.scale];
            let to_world = if pose == [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
                to_world
            } else {
                to_world * native_hooks::hand_pose_matrix(pose)
            };
            // The main hand on its arm, the off hand on the other one.
            let left = self.settings.main_hand_left != (hand == Hand::Off);
            // A Legacy sword held up hides the off hand's shield.
            if hand == Hand::Off && legacy_block {
                continue;
            }
            let Some(stack) = item else {
                // renderArmWithItem: only the main hand's empty arm shows,
                // on the main arm (the left one with the main hand set to
                // left), and not while the player is invisible.
                if hand == Hand::Off || self.own_invisible() {
                    continue;
                }
                // renderPlayerArm
                let m = to_world
                    * sunrise_client::first_person::player_arm_pose(left, inverse_height, attack);
                runs.push((out.len() as u32, 0));
                self.first_person_arm(left, m, packed, a, &mut out);
                continue;
            };
            let item_id = self.rendered_item_id(&stack.item);
            if let Some(id) = maps::map_id(&stack) {
                use sunrise_client::first_person::{MapHold, map_arm_poses, one_handed_map_pose};
                let hold = MapHold::of(hand, off_hand_empty);
                // renderOneHandedMap and renderTwoHandedMap: the arms under
                // the map, unless the player is invisible.
                if !self.own_invisible() {
                    runs.push((out.len() as u32, 0));
                    for (arm, pose) in map_arm_poses(hold, left, self.pitch, inverse_height, attack)
                    {
                        self.first_person_arm(arm, to_world * pose, packed, a, &mut out);
                    }
                }
                runs.push((out.len() as u32, item_id));
                match hold {
                    MapHold::TwoHanded => self.two_handed_map_geometry(
                        to_world,
                        attack,
                        inverse_height,
                        id,
                        packed,
                        &mut out,
                    ),
                    MapHold::OneHanded => self.held_map(
                        to_world * one_handed_map_pose(left, inverse_height, attack),
                        id,
                        packed,
                        &mut out,
                    ),
                }
                continue;
            }
            let mut owner = self.player_owner(Some(&stack));
            // The other hand may hold an equal stack: only the used
            // hand's item is in use, only the main hand's selected.
            owner.using_this_item &= used_hand == Some(hand);
            if hand == Hand::Off {
                owner.is_selected = false;
                owner.is_carried = false;
            }
            // The hand renderer's copy of the main hand stack.
            owner.fishing_cast_with_this_item =
                self.main_hand_rod() && self.cast_rod() == Some(&stack);
            let using = owner.using_this_item && owner.use_item_remaining_ticks > 0;
            let ticks_used =
                owner.use_duration as f32 - (owner.use_item_remaining_ticks as f32 - a + 1.0);
            use item_use::ItemUseState;
            use sunrise_client::first_person::ItemPose;
            let state = using.then(|| self.item_use_state(&stack));
            use sunrise_core::use_animation::UseAnimation;
            let pose = match state {
                Some(ItemUseState::Spear(timing)) => ItemPose::Spear {
                    timing,
                    time_held: ticks_used,
                    ticks_since_hit: self.kinetic_hit_ticks(a),
                },
                Some(ItemUseState::Other(UseAnimation::Bow)) => ItemPose::Bow { ticks_used },
                Some(ItemUseState::Other(UseAnimation::Crossbow)) | None
                    if &*stack.item == "minecraft:crossbow"
                        || state.map(ItemUseState::animation) == Some(UseAnimation::Crossbow) =>
                {
                    let charged = stack.data().is_some_and(|d| !d.charged.is_empty());
                    ItemPose::Crossbow {
                        ticks_used: (using && !charged).then_some(ticks_used),
                        charge_duration: owner.crossbow_charge_duration as f32,
                        // Only the main hand turns a charged crossbow aside.
                        charged: charged && hand == Hand::Main,
                    }
                }
                Some(ItemUseState::Other(UseAnimation::Trident)) => {
                    ItemPose::Trident { ticks_used }
                }
                // ItemUseAnimation.BRUSH: applyBrushTransform.
                Some(ItemUseState::Other(UseAnimation::Brush)) => ItemPose::Brush {
                    remaining: owner.use_item_remaining_ticks as f32,
                    partial: a,
                },
                Some(ItemUseState::Other(animation)) => item_use::use_pose(
                    animation,
                    owner.use_item_remaining_ticks as f32,
                    owner.use_duration as f32,
                    a,
                ),
                _ if spinning => ItemPose::SpinAttack,
                // ItemInHandRenderer: a STAB swing of this hand jabs.
                _ if self
                    .swing
                    .current
                    .is_some_and(|c| c.hand == hand && c.animation.kind == 2) =>
                {
                    ItemPose::Stab
                }
                _ => ItemPose::Swing,
            };
            let mut m = to_world
                * sunrise_client::first_person::item_pose(left, inverse_height, attack, pose);
            // PvP & HUD's shield size (100% is vanilla's), toward the hand holding it.
            let shield_size = self.pvp_shield_size();
            if shield_size != 100 && &*stack.item == "minecraft:shield" {
                m *= glam::Mat4::from_scale(glam::Vec3::splat(f32::from(shield_size) / 100.0));
            }
            // Seeded by the player id plus the context ordinal.
            let (context, seed) = if left {
                (
                    sunrise_assets::item_model::DisplayContext::FirstPersonLeftHand,
                    3,
                )
            } else {
                (
                    sunrise_assets::item_model::DisplayContext::FirstPersonRightHand,
                    4,
                )
            };
            let cit_hand = if hand == Hand::Off {
                sunrise_client::cit::Hand::Off
            } else {
                sunrise_client::cit::Hand::Main
            };
            let state = self.resolve_item_in_hand(&stack, context, Some(&owner), seed, cit_hand);
            // Combat feel Legacy: 1.8.9's pose for a flat item drawn with vanilla's display.
            if self.pvp_legacy_look()
                && !state.layers.is_empty()
                && state
                    .layers
                    .iter()
                    .all(|l| sunrise_client::first_person::is_modern_flat_display(&l.matrix, left))
                && let Some(legacy) = sunrise_client::first_person::legacy_item_pose(
                    left,
                    inverse_height,
                    attack,
                    pose,
                    hand == Hand::Main && legacy_block,
                )
            {
                m = to_world * legacy;
            }
            runs.push((out.len() as u32, item_id));
            let light = self.world.light(
                eye[0].floor() as i32,
                eye[1].floor() as i32,
                eye[2].floor() as i32,
            );
            self.submit_item_world(
                &state,
                m,
                block_entity_render::vertex_light(light),
                &mut out,
            );
        }
        // The hand has its own submit storage, which never uses improved
        // transparency (`LevelRenderer` alone calls `setUseImprovedTransparency`):
        // a held player head stays classic.
        for vertex in &mut out {
            vertex.layer &= !sunrise_client::gfx::OIT_ENTITY;
        }
        (out, runs)
    }
    /// WeatherEffectRenderer: rain and snow columns within the weather radius.
    pub(super) fn weather_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
    ) -> Vec<sunrise_client::weather::Vertex> {
        use sunrise_client::weather;
        let (intensity, _) = self.view_weather(alpha as f32);
        if intensity <= 0.0 {
            return Vec::new();
        }
        let atlas = &self.gui.atlas;
        let (Some(rain_tex), Some(snow_tex)) = (
            atlas.sprites.get("environment/rain").copied(),
            atlas.sprites.get("environment/snow").copied(),
        ) else {
            return Vec::new();
        };
        let (cx, cy, cz) = (
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        );
        // The 32x32 direction table supports at most a 15-block radius.
        let radius = self.settings.weather_radius.clamp(0, 15);
        let eye = camera.to_array();
        let mut rain = Vec::new();
        let mut snow_vertices = Vec::new();
        // Saturation keeps extreme camera coordinates from overflowing the loop.
        for z in cz.saturating_sub(radius)..=cz.saturating_add(radius) {
            for x in cx.saturating_sub(radius)..=cx.saturating_add(radius) {
                let top = self.rain_height(x, z);
                let y0 = cy.saturating_sub(radius).max(top);
                let y1 = cy.saturating_add(radius).max(top);
                if y1 == y0 {
                    continue;
                }
                let Some(snow) = self.precipitation(x, cy, z) else {
                    continue;
                };
                let parameters = self.weather_parameters.get(x, z);
                let light = self.world.light(x, top.max(cy), z);
                let coords = i32::from(light & 15) << 4 | i32::from(light >> 4) << 20;
                // Only light brightening is needed here; UV phases stay zero.
                let lc = if snow {
                    let (block, sky) = (coords >> 4 & 15, coords >> 20 & 15);
                    ((block * 3 + 15) / 4) << 4 | ((sky * 3 + 15) / 4) << 20
                } else {
                    coords
                };
                let column = weather::Column {
                    x,
                    z,
                    bottom_y: y0,
                    top_y: y1,
                    u_offset: 0.0,
                    v_offset: 0.0,
                    light_coords: lc,
                };
                let r = if snow { snow_tex } else { rain_tex };
                let vertices = weather::vertices(
                    &column,
                    parameters,
                    snow,
                    eye,
                    radius,
                    intensity,
                    [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0],
                );
                if snow {
                    snow_vertices.extend(vertices);
                } else {
                    rain.extend(vertices);
                }
            }
        }
        // Vanilla draws every rain column before the snow ones.
        rain.extend(snow_vertices);
        rain
    }
}

// `destroy_stage_0` .. `destroy_stage_9`: the sprite name by stage, so the
// lookup does not format a string per crack.
const DESTROY_STAGES: [&str; 10] = [
    "block/destroy_stage_0",
    "block/destroy_stage_1",
    "block/destroy_stage_2",
    "block/destroy_stage_3",
    "block/destroy_stage_4",
    "block/destroy_stage_5",
    "block/destroy_stage_6",
    "block/destroy_stage_7",
    "block/destroy_stage_8",
    "block/destroy_stage_9",
];
// `ModelBakery.DESTROY_STAGES_OIT`: with improved transparency the cracks are
// ordinary translucent quads (`OIT_CRUMBLING`) and use their own sprites.
const DESTROY_STAGES_OIT: [&str; 10] = [
    "block/destroy_oit_stage_0",
    "block/destroy_oit_stage_1",
    "block/destroy_oit_stage_2",
    "block/destroy_oit_stage_3",
    "block/destroy_oit_stage_4",
    "block/destroy_oit_stage_5",
    "block/destroy_oit_stage_6",
    "block/destroy_oit_stage_7",
    "block/destroy_oit_stage_8",
    "block/destroy_oit_stage_9",
];

/// The sky's frame for a body at `angle`: rotated Y -90, then X by the angle.
fn celestial_frame(angle: f32) -> glam::Mat4 {
    glam::Mat4::from_rotation_y((-90.0f32).to_radians())
        * glam::Mat4::from_rotation_x(angle.to_radians())
}

/// The sun's quad, then the moon's (its phase's sprite `moon`), in `color`, with what each
/// quad is. A body whose sprite is missing is left out, quad and entry alike, so the moon's
/// quad may come first.
fn celestial_bodies(
    sprites: &std::collections::HashMap<String, sunrise_client::hud::Rect>,
    sky: &sunrise_client::environment::Sky,
    moon: &str,
    color: u32,
) -> (Vec<Vertex>, Vec<CelestialBody>) {
    let mut out = Vec::new();
    let mut bodies = Vec::new();
    let mut body = |name: &str, angle: f32, size: f32, kind: CelestialBody| {
        let Some(r) = sprites.get(name).copied() else {
            return;
        };
        bodies.push(kind);
        let moon = kind == CelestialBody::Moon;
        // How brightly HDR output lets the body shine (mesher::hdr_tier):
        // the moon at the low tier, not the sun's.
        let tier = if moon {
            sunrise_client::mesher::LOW
        } else {
            sunrise_client::mesher::VERY_BRIGHT
        };
        let m = celestial_frame(angle);
        let corners = [(-size, -size), (size, -size), (size, size), (-size, size)];
        // buildCelestialQuad (sun) vs buildMoonPhases: the moon's UVs run
        // the other way round.
        let uv = if moon {
            [[r.u1, r.v1], [r.u0, r.v1], [r.u0, r.v0], [r.u1, r.v0]]
        } else {
            [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]]
        };
        for ((x, z), uv) in corners.iter().zip(uv) {
            let p = m.transform_point3(glam::Vec3::new(*x, 100.0, *z));
            out.push(Vertex {
                normal: [0; 4],
                pos: p.to_array(),
                uv,
                layer: 2 | u32::from(tier) << 16,
                color,
                light: 0,

                glint_uv: [0.0, 0.0],
            });
        }
    };
    body(
        "environment/celestial/sun",
        sky.sun_angle,
        30.0,
        CelestialBody::Sun,
    );
    body(moon, sky.moon_angle, 20.0, CelestialBody::Moon);
    (out, bodies)
}

/// The crack sprites drawn: with improved transparency vanilla's own OIT set.
fn crack_stage_names(improved_transparency: bool) -> &'static [&'static str; 10] {
    if improved_transparency {
        &DESTROY_STAGES_OIT
    } else {
        &DESTROY_STAGES
    }
}

/// The crack's sprite size (and stage 0's origin) the crumbling shaders read: of the set drawn,
/// as a resource pack may replace one set and not the other (a 32x pack's `destroy_stage_*`
/// beside vanilla's 16x `destroy_oit_stage_*`, #11164).
pub(super) fn crumbling_sprite(
    sprites: &std::collections::HashMap<String, sunrise_client::hud::Rect>,
    improved_transparency: bool,
) -> [f32; 4] {
    sprites
        .get(crack_stage_names(improved_transparency)[0])
        .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0])
}

#[cfg(test)]
mod crumbling_sprite_tests {
    use super::*;

    #[test]
    fn the_crack_takes_the_size_of_the_set_it_draws() {
        let rect = |u0: f32, size: f32| sunrise_client::hud::Rect {
            u0,
            v0: 0.0,
            u1: u0 + size,
            v1: size,
            w: 16,
            h: 16,
        };
        // A pack's 32x destroy stages beside vanilla's 16x OIT stages, in a 1024-wide atlas.
        let sprites = std::collections::HashMap::from([
            ("block/destroy_stage_0".to_owned(), rect(0.0, 32.0 / 1024.0)),
            (
                "block/destroy_oit_stage_0".to_owned(),
                rect(0.5, 16.0 / 1024.0),
            ),
        ]);
        assert_eq!(
            crumbling_sprite(&sprites, false),
            [0.0, 0.0, 32.0 / 1024.0, 32.0 / 1024.0]
        );
        assert_eq!(
            crumbling_sprite(&sprites, true),
            [0.5, 0.0, 16.0 / 1024.0, 16.0 / 1024.0]
        );
    }
}

/// `celestial_bodies`: each quad comes with what it is, and a body whose sprite is missing
/// leaves out its quad and its entry alike, so the moon alone is the first quad.
#[cfg(test)]
mod celestial_body_tests {
    use super::*;
    use sunrise_client::gfx::CelestialBody::{Moon, Sun};

    const SUN: &str = "environment/celestial/sun";
    const MOON: &str = "environment/celestial/moon/full_moon";

    /// The quads and bodies with a sprite for each of `names`; the sun's sprite starts at u 0.25,
    /// the moon's at 0.5.
    fn bodies(names: &[&str]) -> (Vec<[f32; 2]>, Vec<sunrise_client::gfx::CelestialBody>) {
        let rect = |u0: f32| sunrise_client::hud::Rect {
            u0,
            v0: 0.0,
            u1: u0 + 0.125,
            v1: 0.125,
            w: 16,
            h: 16,
        };
        let sprites = names
            .iter()
            .map(|&name| (name.to_owned(), rect(if name == SUN { 0.25 } else { 0.5 })))
            .collect();
        let sky = sunrise_client::environment::Sky::default();
        let (quads, bodies) = celestial_bodies(&sprites, &sky, MOON, u32::MAX);
        assert_eq!(quads.len(), bodies.len() * 4, "one entry a quad");
        (quads.iter().step_by(4).map(|v| v.uv).collect(), bodies)
    }

    #[test]
    fn the_sun_then_the_moon() {
        // The moon's UVs run the other way round: its first corner is (u1, v1).
        assert_eq!(
            bodies(&[SUN, MOON]),
            (vec![[0.25, 0.0], [0.625, 0.125]], vec![Sun, Moon])
        );
    }

    #[test]
    fn the_moon_alone_is_the_first_quad_and_the_moon() {
        assert_eq!(bodies(&[MOON]), (vec![[0.625, 0.125]], vec![Moon]));
    }

    #[test]
    fn the_sun_alone_is_the_sun() {
        assert_eq!(bodies(&[SUN]), (vec![[0.25, 0.0]], vec![Sun]));
    }

    #[test]
    fn no_sprite_no_body() {
        assert_eq!(bodies(&[]), (vec![], vec![]));
    }
}
