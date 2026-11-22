//! Mobs and players drawn with the mods' render hooks on the client, the pair NeoForge has around
//! a living entity's render: `render_player_pre` (a player's, `RenderPlayerEvent.Pre`) then
//! `render_living_pre` (`RenderLivingEvent.Pre`) before its model, each with the entity's render
//! state and a [`BlockEntityDraw`] posed at its feet and turned with its body, so a mod draws with
//! the helpers block entities draw with; then its model and vanilla layers, or in their place the
//! renderer a mod registered for its type (`register_entity_renderers`); after them, the layers mods added
//! to its type or player model (`register_entity_layers`, `EntityRenderersEvent.AddLayers`), then
//! `render_living_post` and `render_player_post`. A canceled pre hook skips all of the entity's
//! own drawing this frame, the later passes' name tag and leash included: they ask
//! [`Game::render_canceled`], which the frame's entity pass starts afresh. The flames of a burning
//! entity and its shadow are queued by the frame after the entity's drawing and stay (#1285).
pub use mod_api::game::{MobRender, PlayerRender};

use super::{Game, native_block_entity_draw::BlockEntityDraw};
use glam::{DVec3, Mat4};
use mod_api::{EntityRenderState, FrameModel, HookId};
use std::cell::RefCell;
use sunrise_client::{
    block_entity_render::diffuse_lights,
    fast_hash::FxHashSet,
    mesher::Vertex,
    mobs::{Mob, Species, anim, render, villager},
};

/// What the frame's entity pass decided about the mods' render hooks: whether any mod listens to
/// the pair (asked once a frame), and the entities a pre hook canceled.
#[derive(Default)]
struct EntityFrame {
    listened: bool,
    living_pre: bool,
    player_pre: bool,
    living_post: bool,
    player_post: bool,
    canceled: FxHashSet<u32>,
}

thread_local! {
    /// The frame being drawn on this thread.
    static FRAME: RefCell<EntityFrame> = RefCell::default();
}

/// Starts a frame's entity pass: asks once whether any mod listens to the render hook pair, and
/// forgets the last frame's canceled entities.
pub(in crate::game) fn begin_entity_frame() {
    use mod_api::__private::listened;
    let living_post = listened(HookId::RenderLivingPost);
    let player_post = listened(HookId::RenderPlayerPost);
    let living_pre = listened(HookId::RenderLivingPre);
    let player_pre = listened(HookId::RenderPlayerPre);
    FRAME.with_borrow_mut(|frame| {
        frame.listened = living_pre || player_pre || living_post || player_post;
        frame.living_pre = living_pre;
        frame.player_pre = player_pre;
        frame.living_post = living_post;
        frame.player_post = player_post;
        frame.canceled.clear();
    });
}

/// The entity id a mod is told for a drawn mob. This window's own player is
/// drawn from a model made for the frame (`Mob::new(0, ..)`), so it is told
/// `own_id` instead of the model's 0; every other entity carries its own id.
fn render_id(mob_id: u32, local: bool, own_id: i32) -> u32 {
    match local {
        true => u32::try_from(own_id).unwrap_or(mob_id),
        false => mob_id,
    }
}

/// The render state of `m` at `partial` (`LivingEntityRenderer.extractRenderState`),
/// told as entity `id`.
pub(super) fn mob_render<'a>(m: &'a Mob, id: u32, partial: f32) -> MobRender<'a> {
    let body = anim::body_rot(m, partial);
    // The walk cycle, head turn and age as the vanilla models read them:
    // no walking while riding or dead, a baby's legs three times as far
    // (`WalkAnimationState.position(partial)`, `positionScale`).
    let pose = anim::state(m, partial);
    MobRender {
        id,
        kind: &m.kind.id,
        custom_name: m.custom_name.as_deref(),
        baby: m.is_baby(),
        partial,
        age: pose.age,
        body_yaw: body,
        head_yaw: pose.y_rot,
        pitch: pose.x_rot,
        walk_position: pose.pos,
        walk_speed: pose.speed,
        hurt_time: m.hurt_time,
        death_time: m.death_time,
        on_ground: m.ground.on_ground,
    }
}

/// The frame of `m` at `partial` an entity renderer or layer reads, told as entity `id`:
/// `LivingEntityRenderer.extractRenderState`'s `LivingEntityRenderState`.
pub(super) fn entity_render_state(m: &Mob, id: u32, partial: f32) -> EntityRenderState {
    let body_rot = anim::body_rot(m, partial);
    let pose = anim::state(m, partial);
    // `isEntityUpsideDown` turns the head's angles over with the body.
    let upside_down = anim::upside_down(m);
    let flip = if upside_down { -1.0 } else { 1.0 };
    let baby = m.is_baby();
    let living = mod_api::__private::built::LivingRenderState {
        body_rot,
        y_rot: pose.y_rot * flip,
        x_rot: pose.x_rot * flip,
        death_time: if m.death_time > 0 {
            m.death_time as f32 + partial
        } else {
            0.0
        },
        walk_animation_pos: pose.pos,
        walk_animation_speed: pose.speed,
        scale: m.model_scale(),
        age_scale: if baby { 0.5 } else { 1.0 },
        is_upside_down: upside_down,
        is_baby: baby,
        is_in_water: m.fluids.in_water,
        has_red_overlay: render::red_overlay(m),
        pose: m.pose(),
        // A sleeping villager, or a player or mannequin lying in a bed, as the model lays it.
        bed_orientation: (m.species() == Species::Villager || render::avatar(m))
            .then(|| villager::bed_orientation(m))
            .flatten(),
    };
    mod_api::__private::built::EntityRenderState {
        id,
        partial,
        age: pose.age,
        yaw: m.previous_yaw + sunrise_client::mobs::living::wrap(m.yaw - m.previous_yaw) * partial,
        pitch: pose.x_rot,
        living: Some(living.into()),
        tnt_fuse: None,
        orb_value: None,
        painting: None,
        item_frame: None,
        attachments: None,
    }
    .into()
}

/// The pose a mod's mob model starts from: at the feet, `at` (relative to the
/// camera), turned with the body so +z is the way it faces, +x its left and +y
/// up, in blocks.
pub(super) fn mob_pose(at: DVec3, body_yaw: f32) -> Mat4 {
    Mat4::from_translation(at.as_vec3()) * Mat4::from_rotation_y((-body_yaw).to_radians())
}

impl Game<'_> {
    /// Whether a mod listens to the render hook pair this frame (asked once, when the frame's
    /// entity pass began): with none, nothing is posed or built for the hooks per entity.
    pub(super) fn mob_render_listened(&self) -> bool {
        FRAME.with_borrow(|frame| frame.listened)
    }
    /// Whether a Post listener can read the final model of this entity.
    pub(super) fn mob_render_post_listened(&self, player: bool) -> bool {
        FRAME.with_borrow(|frame| frame.living_post || (player && frame.player_post))
    }
    /// Whether a mod's pre hook canceled the drawing of entity `id` this frame: the later passes
    /// that are part of its drawing (its name tag and leash) leave it out too; its flames and
    /// shadow don't ask.
    pub(in crate::game) fn render_canceled(&self, id: u32) -> bool {
        FRAME.with_borrow(|frame| !frame.canceled.is_empty() && frame.canceled.contains(&id))
    }
    /// Whether this window's own player was canceled this frame.
    pub(in crate::game) fn own_render_canceled(&self) -> bool {
        u32::try_from(self.own_entity_id()).is_ok_and(|id| self.render_canceled(id))
    }
    /// The mods' pre hooks for `mob`, before any of its model is drawn, lit by `light` (packed as
    /// the entity pass packs it): `render_player_pre` for a player (`who`: its name, and whether
    /// it is this window's own), then `render_living_pre`, each from a draw posed at the feet and
    /// turned with the body, whose geometry is kept. Both borrow the same selected model before
    /// animation. False means nothing more should draw; when only Living Pre cancels a player,
    /// Player Post is called here without a model. `listened` is the frame's
    /// [`Game::mob_render_listened`].
    #[allow(clippy::too_many_arguments)]
    pub(super) fn mod_render_pre<'m>(
        &self,
        mob: &'m Mob,
        who: Option<PlayerRender<'_>>,
        listened: bool,
        partial: f32,
        camera: DVec3,
        light: u32,
        frame_draw: &mut render::FrameDraw<'m>,
        out: &mut Vec<Vertex>,
    ) -> bool {
        if !listened {
            return true;
        }
        let (living_pre, player_pre, player_post) =
            FRAME.with_borrow(|frame| (frame.living_pre, frame.player_pre, frame.player_post));
        let player_pre = who.as_ref().filter(|_| player_pre);
        if !living_pre && player_pre.is_none() {
            return true;
        }
        let Some(model) = frame_draw.prepare(mob, self.res.mob_data()) else {
            return true;
        };
        let local = who.as_ref().is_some_and(|who| who.local);
        let mut canceled = false;
        if let Some(who) = player_pre {
            self.mod_entity_draw(mob, local, partial, camera, light, out, |render, draw| {
                self.native_render_player_pre(render, who, draw, &mut *model, &mut canceled);
            });
        }
        if !canceled && living_pre {
            self.mod_entity_draw(mob, local, partial, camera, light, out, |render, draw| {
                self.native_render_living_pre(render, draw, &mut *model, &mut canceled);
            });
            if canceled
                && player_post
                && let Some(who) = &who
            {
                self.mod_entity_draw(mob, local, partial, camera, light, out, |render, draw| {
                    self.native_render_player_post(render, who, None, draw);
                });
            }
        }
        if !canceled {
            return true;
        }
        let id = render_id(mob.id, local, self.own_entity_id());
        FRAME.with_borrow_mut(|frame| frame.canceled.insert(id));
        false
    }
    /// The renderer a mod registered for `mob`'s type (`register_entity_renderers`), drawn in place
    /// of its model and its vanilla layers, from a draw at its feet, not turned: true when it
    /// drew. False when none is registered for the type, and when it fails this frame (then it is
    /// asked no more until resources reload): the type draws its own model.
    pub(super) fn mod_living_renderer_geometry(
        &self,
        mob: &Mob,
        partial: f32,
        camera: DVec3,
        light: u32,
        in_level: bool,
        out: &mut Vec<Vertex>,
    ) -> bool {
        let Some(resolved) = super::native_entity_renderers::living_renderer(mob) else {
            return false;
        };
        let entity = entity_render_state(mob, mob.id, partial);
        let feet = mob.previous.lerp(mob.position, f64::from(partial));
        let feet = DVec3::new(feet.x, feet.y, feet.z);
        let block = [feet.x, feet.y, feet.z].map(|c| c.floor() as i32);
        self.mod_entity_renderer_geometry(
            &resolved,
            &entity,
            &mob.kind.id,
            feet - camera,
            in_level.then_some(feet),
            block,
            light,
            out,
        )
    }
    /// The layers mods added to `mob`'s type or player model (`register_entity_layers`), drawn
    /// after its vanilla model and layers: `layers`, looked up once for the mob by `mod_layers`,
    /// whose answer also decided whether the draw kept its model. `who` is set for a player, and
    /// `model` lends the final model as the vanilla draw posed it, if one ran.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn mod_layer_geometry(
        &self,
        mob: &Mob,
        who: Option<PlayerRender<'_>>,
        model: Option<&dyn FrameModel>,
        layers: Option<super::native_entity_layers::Layers>,
        partial: f32,
        camera: DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        if let Some(layers) = layers {
            let local = who.as_ref().is_some_and(|who| who.local);
            let id = render_id(mob.id, local, self.own_entity_id());
            self.registered_layer_geometry(mob, id, &layers, model, partial, camera, light, out);
        }
    }
    /// The mods' post hooks for `mob`, whose pre hooks weren't canceled: `render_living_post`,
    /// then, for a player (`who`), `render_player_post`. Asked whether or not its layers drew (a
    /// spectating player draws none). `model` is the same borrowed model offered to the layers,
    /// absent if there was no vanilla body pose.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn mod_render_post(
        &self,
        mob: &Mob,
        who: Option<PlayerRender<'_>>,
        model: Option<&dyn FrameModel>,
        partial: f32,
        camera: DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        // Every mob in view each frame: only when a mod listens.
        let (living_post, player_post) =
            FRAME.with_borrow(|frame| (frame.living_post, frame.player_post));
        let local = who.as_ref().is_some_and(|who| who.local);
        if living_post {
            self.mod_entity_draw(mob, local, partial, camera, light, out, |render, draw| {
                self.native_render_living_post(render, model, draw);
            });
        }
        if player_post && let Some(who) = &who {
            self.mod_entity_draw(mob, local, partial, camera, light, out, |render, draw| {
                self.native_render_player_post(render, who, model, draw);
            });
        }
    }
    /// Gives `hook` the frame of `mob` and a draw posed at its feet, whose geometry is added to
    /// `out`. `local` marks this window's own player, whose frame carries its own entity id.
    #[allow(clippy::too_many_arguments)]
    fn mod_entity_draw(
        &self,
        mob: &Mob,
        local: bool,
        partial: f32,
        camera: DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
        hook: impl FnOnce(&MobRender<'_>, &mut BlockEntityDraw),
    ) {
        let render = mob_render(mob, render_id(mob.id, local, self.own_entity_id()), partial);
        let a = f64::from(partial);
        let feet = mob.previous.lerp(mob.position, a);
        let feet = DVec3::new(feet.x, feet.y, feet.z);
        let mut draw = BlockEntityDraw::posed(
            partial,
            self.world.game_time,
            light,
            [feet.x, feet.y, feet.z].map(|c| c.floor() as i32),
            mob_pose(feet - camera, render.body_yaw),
            diffuse_lights(self.nether_lighting()),
            std::mem::take(out),
        )
        .animated(self.animation_time());
        hook(&render, &mut draw);
        *out = draw.into_vertices();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn this_windows_player_is_told_its_own_entity_id() {
        // The frame's model has id 0; the server gave this window 4321.
        assert_eq!(render_id(0, true, 4321), 4321);
    }

    #[test]
    fn every_other_entity_keeps_its_own_id() {
        assert_eq!(render_id(90, false, 4321), 90);
        assert_eq!(render_id(0, false, 4321), 0);
    }

    #[test]
    fn a_missing_own_id_leaves_the_models_id() {
        assert_eq!(render_id(7, true, -1), 7);
    }

    #[test]
    fn the_pose_faces_where_the_body_faces() {
        // Entity yaw 0 faces south (+z), 90 west (-x), 180 north, 270 east.
        for (yaw, ahead, left) in [
            (0.0, Vec3::Z, Vec3::X),
            (90.0, Vec3::NEG_X, Vec3::Z),
            (180.0, Vec3::NEG_Z, Vec3::NEG_X),
            (270.0, Vec3::X, Vec3::NEG_Z),
        ] {
            let pose = mob_pose(DVec3::ZERO, yaw);
            assert!(close(pose.transform_vector3(Vec3::Z), ahead), "{yaw}");
            assert!(close(pose.transform_vector3(Vec3::X), left), "{yaw}");
            assert!(close(pose.transform_vector3(Vec3::Y), Vec3::Y), "{yaw}");
        }
    }

    #[test]
    fn the_pose_starts_at_the_feet() {
        let at = DVec3::new(3.0, -1.5, 0.25);
        let pose = mob_pose(at, 37.0);
        assert!(close(pose.transform_point3(Vec3::ZERO), at.as_vec3()));
    }
}
