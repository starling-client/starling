//! Baked equipment overrides, drawn in the same pose and material path as vanilla layers.
use super::{ModelNormals, Part, Poses, Quads, Vertex, anim, posed, push_polygons, put};
use crate::mobs::{
    Polygon,
    skin::{Paint, Skin},
};
use glam::Mat4;
use mod_api::ModelPart;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub enum ArmorSlot {
    Head,
    Chest,
    Legs,
    Feet,
}

#[derive(Clone, Copy)]
pub enum WornModel<'a> {
    Vanilla(&'a Arc<Part>),
    Override(&'a EquipmentModel, ArmorSlot),
}

pub type WornLayer<'a> = (WornModel<'a>, [f32; 4], u32, u32, bool);

impl WornModel<'_> {
    /// Submit a worn model. A registered override already holds an allocation-free CPU layout.
    pub fn submit(
        self,
        skin: &mut Option<&mut Skin<'_>>,
        base: Mat4,
        poses: &Poses,
        paint: Paint,
        out: &mut Vec<Vertex>,
    ) {
        match self {
            Self::Vanilla(part) => put(
                skin,
                part,
                base,
                poses,
                paint.uv,
                paint.scroll,
                paint.color,
                paint.light,
                paint.overlay,
                out,
            ),
            Self::Override(model, slot) => {
                let mut transforms = model
                    .transforms
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut at = 0;
                while let Some(part) = model.parts.get(at) {
                    let pose = poses.pose_by_key(part.key).copied().unwrap_or(part.pose);
                    if part.slots & (1 << slot as u8) == 0
                        || poses.is_hidden_key(part.key)
                        || pose[6] == 0.0 && pose[7] == 0.0 && pose[8] == 0.0
                    {
                        at = part.end;
                        continue;
                    }
                    let (parent, normals) = part.parent.map_or_else(
                        || (base, ModelNormals::new(base)),
                        |parent| transforms[parent],
                    );
                    let m = posed(parent, pose);
                    let normals = normals.part(pose);
                    transforms[at] = (m, normals);
                    if !poses.skips_key(part.key) {
                        push_polygons(&part.polygons, m, normals, paint, &mut Quads::new(out));
                    }
                    at += 1;
                }
            }
        }
    }
}

/// An owned flat preorder model; animating or dropping it never walks the Rust call stack.
pub struct EquipmentModel {
    parts: Vec<BakedPart>,
    // Per-draw parent transforms, allocated alongside the bake. Not a cache: each draw fills
    // every visited parent before its children, including when multiple entities share a model.
    transforms: Mutex<Vec<(Mat4, ModelNormals)>>,
}

struct BakedPart {
    parent: Option<usize>,
    end: usize,
    key: u64,
    pose: [f32; 9],
    slots: u8,
    polygons: Vec<Polygon>,
}

impl EquipmentModel {
    /// Validate the mod's input once before publishing the model. Preserve `ModelPart.render`'s
    /// preorder and parent transforms: invisibility skips the subtree, `skipDraw` just the faces.
    pub fn bake(root: &ModelPart) -> Result<Self, &'static str> {
        use ArmorSlot::{Chest, Feet, Head, Legs};
        let mut parts = Vec::new();
        let mut pending = vec![(None, "root", root)];
        while let Some((parent, name, model)) = pending.pop() {
            let pose = [
                model.x,
                model.y,
                model.z,
                model.x_rot,
                model.y_rot,
                model.z_rot,
                model.x_scale,
                model.y_scale,
                model.z_scale,
            ];
            if !pose.iter().all(|value| value.is_finite()) {
                return Err("non-finite model pose");
            }
            let mut polygons = Vec::new();
            for polygon in &model.polygons {
                if !polygon.normal.iter().all(|x| x.is_finite())
                    || !polygon
                        .vertices
                        .iter()
                        .all(|v| v.pos.iter().chain([&v.u, &v.v]).all(|x| x.is_finite()))
                {
                    return Err("non-finite model geometry");
                }
                if model.visible && !model.skip_draw {
                    polygons.push(Polygon {
                        normal: polygon.normal,
                        vertices: polygon
                            .vertices
                            .map(|v| [v.pos[0], v.pos[1], v.pos[2], v.u, v.v]),
                        cull_back: false,
                    });
                }
            }
            let at = parts.len();
            if model.visible {
                let start = pending.len();
                pending.extend(
                    model
                        .children()
                        .map(|(name, child)| (Some(at), name, child)),
                );
                pending[start..].reverse();
            }
            let slots = match name {
                _ if !model.visible => 0,
                "head" | "hat" => 1 << Head as u8,
                "body" => (1 << Chest as u8) | (1 << Legs as u8),
                "left_arm" | "right_arm" => 1 << Chest as u8,
                "left_leg" | "right_leg" => (1 << Legs as u8) | (1 << Feet as u8),
                _ => u8::MAX,
            };
            parts.push(BakedPart {
                parent,
                end: at + 1,
                key: anim::key(name),
                pose,
                slots,
                polygons,
            });
        }
        // All parents precede their descendants, so a reverse pass finds each subtree end.
        for at in (1..parts.len()).rev() {
            let child = &parts[at];
            if let Some(parent) = child.parent {
                parts[parent].end = parts[parent].end.max(child.end);
            }
        }
        let transforms = vec![(Mat4::IDENTITY, ModelNormals::new(Mat4::IDENTITY)); parts.len()];
        Ok(Self {
            parts,
            transforms: Mutex::new(transforms),
        })
    }
}
