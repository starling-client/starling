//! The models a client mod draws as part of a block (`Chunk::extra_models`,
//! `ClientRenderClientHost::set_block_extra_models`): drawn after the section's own geometry and
//! held to the section's quad and byte budget (`mod_geometry::MAX_QUADS`, `MAX_SECTION_BYTES`),
//! so no list of models can grow a section's mesh past what its upload holds.
//!
//! A mod's own tint source (`BlockTintSource`) colouring an extra model is asked on the thread
//! that schedules the mesh ([`Mesher::prepare_attached_tints`]), a failure contained there:
//! the workers read the prepared colours and call no mod code for an extra model.
use super::mod_geometry::{Budget, QUAD_BYTES};
use super::{
    CellFaces, GroupCells, MAX_OVERLAYS, MeshScratch, Mesher, Padded, SectionMesh,
    SectionVisibilityPlan, TerrainSource, Tints, visibility_plan::NoMasks,
};
use crate::{chunk::Chunk, fast_hash::FxHashMap};
use std::sync::Arc;

/// The section's extra-model colours from custom sources, keyed by section-local position,
/// state and tint index. A failure also disables untaken source calls in the base mesh: a
/// *different* provider can share state (such as a poisoned mutex) with the failed one.
#[derive(Debug, Default)]
pub struct AttachedTints {
    pub(super) colors: FxHashMap<([i32; 3], u32, i32), u32>,
    pub(super) failed: bool,
}

/// Reused assembly buffers for the scheduling thread's tint pass. The pass shares actual face
/// emission and budget accounting with the worker, so only faces that fit ask a source.
#[derive(Default)]
pub(crate) struct AttachedTintScratch {
    mesh: SectionMesh,
    scratch: MeshScratch,
}

impl Mesher<'_> {
    pub(crate) fn prepare_attached_tints(
        &mut self,
        region: &[[Option<&Chunk>; 3]; 3],
        sy: i32,
        visibility_bytes: Option<usize>,
        plan: Option<&SectionVisibilityPlan>,
        preparation: &mut AttachedTintScratch,
    ) -> Option<Arc<AttachedTints>> {
        let out = std::mem::take(&mut preparation.mesh);
        let scratch = &mut preparation.scratch;
        preparation.mesh = match plan {
            Some(plan) if plan.hides_terrain() => out,
            Some(plan) => self.mesh_masked::<_, true, true>(
                region,
                sy,
                out,
                scratch,
                plan,
                visibility_bytes.unwrap_or_default(),
            ),
            None => match visibility_bytes {
                Some(bytes) => {
                    self.mesh_masked::<_, true, true>(region, sy, out, scratch, &NoMasks, bytes)
                }
                None => self.mesh_masked::<_, false, true>(region, sy, out, scratch, &NoMasks, 0),
            },
        };
        scratch.attached_tints.take()
    }
}

/// The most quads one model face becomes: its `ctm_compact` pieces (up to 3 by 3), and as many
/// again for each connected-texture overlay drawn over it.
pub(super) const FACE_QUADS: usize = 9 * (1 + MAX_OVERLAYS);
/// The most mesh bytes one quad takes: [`QUAD_BYTES`] and its shader-pack source.
pub(super) const QUAD_MAX_BYTES: usize = QUAD_BYTES + size_of::<TerrainSource>();

/// Draws `cells`' extra models into `out`, after the block geometry of the whole section, in
/// cell order and then list order. A face is drawn only while the section's budget still holds
/// [`FACE_QUADS`] more quads of [`QUAD_MAX_BYTES`], whatever it expands to; at the first face
/// that cannot fit, drawing stops, so the last model may be partially drawn. The block geometry
/// is always kept. During preparation, the faces that fit ask their custom tint sources; on the
/// worker they use the prepared colour or white without calling mod code.
#[allow(clippy::too_many_arguments)]
pub(super) fn add(
    mesher: &Mesher<'_>,
    pad: &Padded,
    groups: Option<&GroupCells>,
    tints: &mut Tints<'_>,
    out: &mut SectionMesh,
    cells: &[(CellFaces, u32)],
    models: &FxHashMap<u32, Arc<[mod_api::ModelRef]>>,
    visibility_bytes: usize,
) {
    tints.drawing_extras = true;
    for (at, key) in cells {
        let state = &mesher.tables.info[at.sample.id as usize];
        let start = [out.solid.len(), out.translucent.len()];
        let mut full = false;
        'models: for &model in models[key].iter() {
            // A handle a resource reload made stale draws nothing.
            let Ok(model) = mesher.render.resolve_model(model) else {
                continue;
            };
            for face in &model.faces {
                if !fits(out, start, mesher.shader_pack, visibility_bytes) {
                    full = true;
                    break 'models;
                }
                mesher.model_face(
                    pad,
                    groups,
                    tints,
                    out,
                    at,
                    state,
                    face,
                    super::Pass::Own { force_solid: false },
                );
            }
        }
        if mesher.shader_pack {
            let source = TerrainSource::new(at.sample.id, at.sample.pos, state.emission, false);
            out.record_source(start, source);
        }
        if full {
            break;
        }
    }
    tints.drawing_extras = false;
}

/// Whether the section `out` has room for one more face at its largest. With a shader pack, the
/// quads drawn since `start` take their sources once the cell is done: counted now.
fn fits(out: &SectionMesh, start: [usize; 2], shader_pack: bool, visibility_bytes: usize) -> bool {
    let mut left = Budget::left_after(out, visibility_bytes);
    if shader_pack {
        let pending = (out.solid.len() - start[0] + out.translucent.len() - start[1]) / 4;
        left.bytes = left
            .bytes
            .saturating_sub(pending * size_of::<TerrainSource>());
    }
    left.quads >= FACE_QUADS && left.bytes >= FACE_QUADS * QUAD_MAX_BYTES
}
