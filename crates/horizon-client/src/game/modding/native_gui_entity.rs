//! `DrawContext::entity`: a living entity a mod draws in a GUI box, as vanilla's
//! `InventoryScreen.renderEntityInInventory` draws the inventory's player: still, facing out,
//! full bright and lit by `Lighting.ENTITY_IN_UI`, with a depth of its own, cut off at the box.
use super::{Game, commands::Target, perspective};
use horizon_client::{hud::HudBatch, mobs::Mob};
use horizon_core::nbt::{Compound, JavaString, Tag};
use mod_api::{GuiEntity, GuiEntityError, GuiEntityLook, GuiEntityView};
use std::{cell::RefCell, sync::Arc};

/// How many entities made for screens the game keeps: a screen that draws the same few every
/// frame makes each once, as one that keeps its `LivingEntity` does.
const KEPT: usize = 8;

/// An entity made for a screen from its type and data.
struct Made {
    kind: String,
    data: Compound,
    mob: Mob,
}

thread_local! {
    /// The entities made last, the newest first.
    static MADE: RefCell<Vec<Made>> = const { RefCell::new(Vec::new()) };
}

/// The view draws something: a box with room in it, a positive size, finite numbers.
fn drawable(view: &GuiEntityView) -> bool {
    let [x0, y0, x1, y1] = view.rect;
    let look = match view.look {
        GuiEntityLook::At([x, y]) => x.is_finite() && y.is_finite(),
        GuiEntityLook::Turned { yaw, pitch } => yaw.is_finite() && pitch.is_finite(),
        _ => false,
    };
    x0 < x1
        && y0 < y1
        && view.rect.iter().all(|v| v.is_finite())
        && view.size.is_finite()
        && view.size > 0.0
        && view.offset_y.is_finite()
        && look
}

/// `kind` as a namespaced id, as `/summon` reads an entity's `id`.
fn namespaced(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

impl Game<'_> {
    /// `DrawContext::entity` on the HUD batch `batch`.
    pub(in crate::game) fn native_gui_entity(
        &self,
        batch: &mut HudBatch,
        entity: &GuiEntity,
        view: GuiEntityView,
    ) -> Result<(), GuiEntityError> {
        if !drawable(&view) {
            return Err(GuiEntityError::BadView);
        }
        if self.res.mob_types().is_empty() {
            return Err(GuiEntityError::Unavailable);
        }
        match entity {
            GuiEntity::Kind { kind, data } => MADE.with_borrow_mut(|made| {
                let at = self.made_entity(made, kind, data)?;
                self.gui_entity_preview(&made[at].mob, &[], batch, view);
                Ok(())
            }),
            GuiEntity::Existing(handle) => match self.native_client_entity_target(*handle) {
                Some(Target::Player) => {
                    let m = self.player_model.as_ref().ok_or(GuiEntityError::NoEntity)?;
                    self.gui_entity_preview(m, &self.player_armor_layers(), batch, view);
                    Ok(())
                }
                Some(Target::Mob(id)) => {
                    let m = self.mobs.iter().find(|m| m.id == id && !m.removed);
                    let m = m.ok_or(GuiEntityError::NoEntity)?;
                    self.gui_entity_preview(m, &[], batch, view);
                    Ok(())
                }
                _ => Err(GuiEntityError::NoEntity),
            },
            _ => Err(GuiEntityError::Unavailable),
        }
    }

    /// Where in `made` the entity of `kind` loaded from `data` is: one kept from an earlier
    /// call, else one made now and kept first, the oldest let go past [`KEPT`].
    fn made_entity(
        &self,
        made: &mut Vec<Made>,
        kind: &str,
        data: &Compound,
    ) -> Result<usize, GuiEntityError> {
        let kind = namespaced(kind);
        let Some(t) = self.res.mob_types().get(&kind) else {
            return Err(GuiEntityError::UnknownType);
        };
        // A resource reload makes the types again: an entity of an old one is made again.
        if let Some(at) = made
            .iter()
            .position(|m| m.kind == kind && m.data == *data && Arc::ptr_eq(&m.mob.kind, t))
        {
            return Ok(at);
        }
        let mob = if data.is_empty() {
            Mob::new(u32::MAX, t.clone(), horizon_core::geometry::Vec3::ZERO, 0.0)
        } else {
            match data.get(&JavaString::from("id")) {
                Some(Tag::String(id)) if namespaced(&id.to_string_lossy()) != kind => {
                    return Err(GuiEntityError::BadData);
                }
                Some(Tag::String(_)) | None => {}
                Some(_) => return Err(GuiEntityError::BadData),
            }
            let mut tag = data.clone();
            tag.insert("id".into(), Tag::String(kind.as_str().into()));
            horizon_client::entity_nbt::mob_from_nbt(
                &Tag::Compound(tag),
                self.res.mob_types(),
                u32::MAX,
                Some(self.pack.assets.registry),
            )
            .filter(|m| &*m.kind.id == kind.as_str())
            .ok_or(GuiEntityError::BadData)?
        };
        made.insert(
            0,
            Made {
                kind,
                data: data.clone(),
                mob,
            },
        );
        made.truncate(KEPT);
        Ok(0)
    }

    /// `renderEntityInInventory`: `m` still, facing out and turned as `view.look` says, in
    /// `view.rect`, `view.size` pixels a block; `armour` the layers posed like its body (the
    /// local player's own).
    fn gui_entity_preview(
        &self,
        m: &Mob,
        armour: &[(&horizon_client::mobs::Part, [f32; 4], u32, u32, bool)],
        batch: &mut HudBatch,
        view: GuiEntityView,
    ) {
        let [x0, y0, x1, y1] = view.rect;
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        // The body, the head's yaw and pitch, and the camera's tilt: as
        // `renderEntityInInventoryFollowsMouse` sets them toward a point, else the mod's turn,
        // counterclockwise from facing the viewer, with no tilt.
        let (body, head, pitch, tilt) = match view.look {
            GuiEntityLook::At([x, y]) => {
                let x_angle = ((cx - x) / 40.0).atan();
                let y_angle = ((cy - y) / 40.0).atan();
                let body = 180.0 + x_angle * 20.0;
                (body, body + x_angle * 20.0, -y_angle * 20.0, y_angle * 20.0)
            }
            GuiEntityLook::Turned { yaw, pitch } => (180.0 - yaw, 180.0 - yaw, pitch, 0.0),
            _ => return,
        };
        let mut m = m.clone();
        m.previous = m.position;
        m.body_yaw = body;
        m.previous_body_yaw = body;
        m.head_yaw = head;
        m.previous_head_yaw = head;
        m.head_pitch = pitch;
        m.previous_head_pitch = pitch;
        m.hurt_time = 0;
        m.death_time = 0;
        let camera = glam::DVec3::new(m.position.x, m.position.y, m.position.z);
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let mut verts = Vec::new();
        // A screen preview: custom entity model animations see is_in_gui = 1.
        horizon_client::mobs::cem_anim::in_gui(|| {
            horizon_client::mobs::render::render_with(
                &m,
                1.0,
                camera,
                &lookup,
                240 | 240 << 8,
                self.res.mob_data(),
                armour,
                &mut verts,
            );
        });
        // PictureInPictureRenderer: the box centre, (size, size, -size); then the
        // translation and Rz(pi) * Rx(tilt).
        let gui = glam::Mat4::from_translation(glam::Vec3::new(cx, cy, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::new(view.size, view.size, -view.size))
            * glam::Mat4::from_translation(glam::Vec3::new(
                0.0,
                m.height() / 2.0 + view.offset_y,
                0.0,
            ))
            * glam::Mat4::from_rotation_z(std::f32::consts::PI)
            * glam::Mat4::from_rotation_x(tilt.to_radians());
        let glints = perspective::preview_glint_quads(&verts, gui, 0..0, self.glint_frame);
        let quads = perspective::player_preview_quads(verts, gui, None, &atlas.sprites);
        perspective::emit_preview(batch, quads, glints, view.rect);
    }
}
