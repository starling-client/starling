//! One living entity's draw (`LivingEntityRenderer.submit` and the layers, hooks, outline
//! and beam around it). The ordered entity pass shares one `LivingFrame` across its mob runs.
use super::*;
use sunrise_client::mobs::render::FrameDraw;

/// What the mob pass works out once a frame for every living entity it draws.
#[derive(Clone, Copy)]
pub(super) struct LivingFrame<'a> {
    pub(super) camera: glam::DVec3,
    pub(super) alpha: f64,
    /// `Entity.shouldRenderAtSqrDistance`'s view scale.
    pub(super) scale: f64,
    /// Who watches (`Entity.isInvisibleTo`): the local player's team.
    pub(super) viewer: Option<&'a sunrise_client::scoreboard::Team>,
    pub(super) skinning: bool,
    pub(super) listened: bool,
}

impl Game<'_> {
    /// Draws `mob` into `out` as the frame's mob pass does, or nothing when it is out of range,
    /// out of view, hidden or culled.
    pub(super) fn living_geometry<const PRECHECK: bool>(
        &self,
        mob: &Mob,
        frame: &LivingFrame<'_>,
        out: &mut Vec<Vertex>,
    ) {
        let LivingFrame {
            camera,
            alpha,
            scale,
            viewer,
            skinning,
            listened,
        } = *frame;
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        // LevelRenderer: the entity the view looks out of is not drawn
        // in first person.
        if self.hides_as_camera(mob.id) {
            return;
        }
        let is_player = &*mob.kind.id == "minecraft:player";
        let d = glam::DVec3::new(mob.position.x, mob.position.y, mob.position.z) - camera;
        // Entity.shouldRenderAtSqrDistance: 64 times the bounding box's
        // mean edge, scaled by the view scale. RemotePlayer's box counts
        // ten times.
        let (w, h, _) = mob.dims();
        let edge = (w + h + w) / 3.0 * if is_player { 10.0 } else { 1.0 };
        let range = edge * 64.0 * scale;
        if d.length_squared() >= range * range {
            return;
        }
        let beam = sunrise_client::mobs::guardian::beam(mob, alpha as f32);
        // EntityRenderer.shouldRender: the renderer's interpolated culling box,
        // grown by 0.5 (GuardianRenderer also draws while its beam is out).
        if let Some(view) = &self.view_frustum
            && beam.is_none()
        {
            let (lo, hi) = super::super::entity_culling::cull_box(mob, alpha);
            let (lo, hi) = (
                (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
                (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
            );
            if !view.intersects(lo, hi) {
                return;
            }
        }
        let outline = self.mob_outline(mob);
        // Entity.DATA_SHARED_FLAGS_ID invisible (the flag the invisibility
        // effect sets): only the name tag draws (and the outline of a
        // glowing mob, LivingEntityRenderer.getRenderType). A player keeps
        // its armour, elytra and held items (`LivingEntityRenderer.submit`
        // skips only the body).
        let invisible = mob.is_invisible();
        // Entity.isInvisibleTo(viewer): a spectator sees it anyway, the
        // body forced translucent (`LivingEntityRenderer.submit`).
        let seen_invisible = invisible
            && !sunrise_client::scoreboard::invisible_to(
                self.mob_team_of(mob),
                viewer,
                self.spectator(),
                true,
            );
        let layers_only = invisible && outline.is_none() && is_player && !seen_invisible;
        if invisible && outline.is_none() && !layers_only && !seen_invisible {
            return;
        }
        let culling = if PRECHECK
            && outline.is_none()
            && !is_player
            && mob.kind.id.starts_with("minecraft:")
            && (0..6).all(|slot| mob.extra.slot_stack(slot).is_none())
            && beam.is_none()
        {
            self.entity_cull_decision(mob, alpha)
        } else {
            EntityCullDecision::CheckVertices
        };
        if culling.hides_entity() {
            return;
        }
        let entity_start = out.len();
        self.mark_motion(entity_start, (1 << 32) | u64::from(mob.id));
        self.mark_entity(entity_start, &mob.kind.id);
        self.rendered_mobs.set(self.rendered_mobs.get() + 1);
        let packed = self.mob_packed_light(mob, alpha);
        // The mods' pre hooks (a player's `render_player_pre`, then `render_living_pre`),
        // before any of the vanilla model's parts (a sulfur cube's swallowed block) are
        // drawn. A cancel skips the rest of the entity's drawing and its post hooks; the
        // frame's later name tag and leash passes leave it out too, its flames and shadow stay.
        let who = is_player
            .then(|| self.remote_player_name(mob.id))
            .flatten()
            .map(|name| PlayerRender { name, local: false });
        let mut frame_draw = FrameDraw::default();
        if !self.mod_render_pre(
            mob,
            who,
            listened,
            alpha as f32,
            camera,
            packed,
            &mut frame_draw,
            out,
        ) {
            // What the mods drew in its place is culled as the entity's own would be.
            self.cull_entity_vertices(out, entity_start);
            return;
        }
        // A mod's renderer for the type (`register_entity_renderers`) draws in place of its
        // model and vanilla layers; the mods' layers and post hooks still follow.
        let modded =
            self.mod_living_renderer_geometry(mob, alpha as f32, camera, packed, true, out);
        if mob.species() == Species::SulfurCube && !mob.is_invisible() && !modded {
            self.sulfur_cube_body_geometry(
                mob,
                alpha as f32,
                sunrise_client::mobs::render::base_transform(mob, alpha as f32, camera),
                packed,
                out,
            );
        }
        // Other players on a server: PlayerRenderer's HumanoidArmorLayer.
        let armor = if is_player {
            self.worn_armor_layers(mob)
        } else {
            Vec::new()
        };
        let body = out.len();
        // The GPU path: an ordinary mob, no layers the CPU path changes after drawing them
        // (a player's, the HDR tiers below, the sulfur cube's shell). A glowing mob's
        // silhouette is posed too (`SkinFrame::outline_since`), unless it is invisible:
        // then only the outline draws.
        let silhouette_on_gpu = outline.is_none() || !invisible;
        let posable = skinning
            && !(is_player || modded)
            && !layers_only
            && !seen_invisible
            && silhouette_on_gpu
            && armor.is_empty()
            && !matches!(
                mob.species(),
                Species::Blaze | Species::MagmaCube | Species::GlowSquid | Species::SulfurCube
            );
        // AvatarRenderer.shouldRenderLayers: a spectator draws no layers.
        let layers = sunrise_client::mobs::render::layers_shown(mob);
        let added = if layers { self.mod_layers(mob) } else { None };
        let mut skinned = None;
        frame_draw.capture_model = !modded
            && (added.is_some() || (listened && self.mob_render_post_listened(who.is_some())));
        if posable {
            let mut models = self.skin_models.borrow_mut();
            let mut frame = self.skin_frame.borrow_mut();
            let mark = frame.mark();
            let mut skin = sunrise_client::mobs::skin::Skin::new(&mut models, &mut frame);
            sunrise_client::mobs::render::render_skinned(
                mob,
                alpha as f32,
                camera,
                &lookup,
                packed,
                self,
                &mut frame_draw,
                out,
                &mut skin,
            );
            skinned = Some((mark, skin.bounds));
        } else if !modded {
            if seen_invisible {
                sunrise_client::mobs::render::render_seen_invisible(
                    mob,
                    alpha as f32,
                    camera,
                    &lookup,
                    packed,
                    self,
                    &armor,
                    &mut frame_draw,
                    out,
                );
            } else if layers_only {
                sunrise_client::mobs::render::render_layers(
                    mob,
                    alpha as f32,
                    camera,
                    &lookup,
                    packed,
                    self,
                    &armor,
                    &mut frame_draw,
                    out,
                );
            } else {
                sunrise_client::mobs::render::render_with_frame(
                    mob,
                    alpha as f32,
                    camera,
                    &lookup,
                    packed,
                    self,
                    &armor,
                    &mut frame_draw,
                    out,
                );
            }
        }
        let model = frame_draw.model();
        // HDR output: mobs that are light shine by their kind, as blocks do (entity.wgsl
        // fs_hdr_mark); a tier's window lifts only their bright texels.
        if let Some(tier) = match mob.species() {
            Species::Blaze => Some(sunrise_client::mesher::BRIGHT),
            Species::MagmaCube | Species::GlowSquid => Some(sunrise_client::mesher::LOW),
            _ => None,
        } {
            for v in &mut out[body..] {
                v.layer |= u32::from(tier) << 16;
            }
        }
        if is_player && layers {
            // PlayerRenderer's CapeLayer, WingsLayer, ParrotOnShoulderLayer
            // and SpinAttackEffectLayer. Only the cape hides with the body;
            // the others have no `isInvisible` check.
            let chest = mob.extra.slot_id(sunrise_items::equipment::CHEST);
            // `CapeLayer` and `Deadmau5EarsLayer`: `!state.isInvisible`, whoever sees the body.
            if !invisible {
                self.player_cape_geometry(
                    mob,
                    chest,
                    camera,
                    alpha as f32,
                    (
                        self as &dyn sunrise_client::mobs::render::RenderDataSource,
                        packed,
                    ),
                    out,
                );
            }
            if !invisible
                && let Some((_, name)) = self
                    .remote_player_names()
                    .into_iter()
                    .find(|(id, _)| *id == mob.id)
            {
                self.player_ears_geometry(mob, &name, camera, alpha as f32, packed, out);
            }
            self.player_elytra_geometry(mob, chest, camera, alpha as f32, packed, out);
            self.shoulder_parrots_geometry(mob, camera, alpha as f32, packed, out);
            self.spin_attack_geometry(mob, camera, alpha as f32, packed, out);
            self.stuck_in_body_geometry(mob, mob.id as i32, camera, alpha as f32, packed, out);
        }
        // CustomHeadLayer (a pumpkin, skull or head item) included.
        if layers {
            if !modded {
                // Held items join the GPU path's draws, their silhouette too.
                let skin = (skinning && silhouette_on_gpu).then_some(&mut skinned);
                self.mob_block_geometry(mob, camera, alpha, packed, skin, out);
            }
            self.mod_layer_geometry(mob, who, model, added, alpha as f32, camera, packed, out);
        }
        self.mod_render_post(mob, who, model, alpha as f32, camera, packed, out);
        if let Some(rgb) = outline {
            self.mark_outline(&out[entity_start..], rgb);
            if let Some(((first, _, _), _)) = skinned {
                let color = sunrise_client::mobs::render::abgr(rgb);
                self.skin_frame.borrow_mut().outline_since(first, color);
            }
            if invisible && !seen_invisible {
                out.truncate(entity_start);
                return;
            }
        }
        // Glowing outlines and a guardian's beam remain visible independently
        // of whether the body is hidden behind terrain.
        if outline.is_none() && culling.checks_vertices() && beam.is_none() {
            // What the GPU draws of the mob counts by its bounds' corners.
            self.cull_skinned(out, entity_start, skinned);
        }
        // GuardianRenderer: the laser to the locked target.
        if let Some((time, scale)) = beam
            && !modded
            && let Some(target) = self.beam_target(mob.ai.target, alpha)
            && let Some(uv) = lookup("entity/guardian/guardian_beam")
        {
            let a = alpha;
            let eye = glam::DVec3::new(
                mob.previous.x + (mob.position.x - mob.previous.x) * a,
                mob.previous.y + (mob.position.y - mob.previous.y) * a + mob.eye_height(),
                mob.previous.z + (mob.position.z - mob.previous.z) * a,
            ) - camera;
            sunrise_client::mobs::guardian::beam_geometry(
                eye.as_vec3(),
                (target - camera).as_vec3(),
                time,
                scale,
                uv,
                out,
            );
        }
    }
}
