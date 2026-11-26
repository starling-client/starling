//! GPU-driven draws (`WorldDrawContext::draw_indirect` and its stage and offscreen forms):
//! vanilla's `RenderPass.drawIndexedIndirect(GpuBufferSlice commands, int drawCount)` and
//! `drawIndirect`, whose commands a compute shader wrote into a `GpuBuffer` created with
//! `USAGE_INDIRECT_PARAMETERS`.

use crate::{DrawBindings, MeshRef, RenderPipelineRef, StorageBufferRef};

/// Bytes of one non-indexed indirect command: `vertexCount, instanceCount, firstVertex,
/// firstInstance`, four `u32` (vanilla `RenderPass.INDIRECT_DRAW_SIZE`).
pub const INDIRECT_DRAW_SIZE: u64 = 16;

/// Bytes of one indexed indirect command: `indexCount, instanceCount, firstIndex, baseVertex,
/// firstInstance`, four `u32` and an `i32` (vanilla `RenderPass.INDIRECT_INDEXED_DRAW_SIZE`).
pub const INDIRECT_INDEXED_DRAW_SIZE: u64 = 20;

/// A draw whose commands are read from a [`StorageBufferRef`] on the GPU, for
/// [`crate::WorldDrawContext::draw_indirect`], [`crate::PrepareRenderBuffersContext::draw_indirect`]
/// and [`crate::OffscreenDrawContext::draw_indirect`].
///
/// The engine draws exactly `max_count` commands from `indirect_offset` (vanilla's
/// `drawIndexedIndirect(commands, drawCount)`), the same on every backend. A command the mod's
/// culling shader does not want drawn is left in place with `instance_count = 0`, which draws
/// nothing. Where [`crate::RenderCapabilities::multi_draw_indirect_count`] is set, a compute pass may
/// instead compact the commands and write how many there are into a count buffer
/// ([`Self::with_count_buffer`]): the engine then draws that many, at most `max_count`
/// (`multi_draw_indirect_count`), so culled slots cost nothing.
///
/// The command layout follows `mesh`: with a [`MeshRef`] that has an index buffer, each command is
/// an indexed one of [`INDIRECT_INDEXED_DRAW_SIZE`] bytes and the index format is the mesh's
/// own; otherwise (no mesh, or a mesh without indices) each is a non-indexed one of
/// [`INDIRECT_DRAW_SIZE`] bytes. A non-indexed draw takes a pipeline whose topology is
/// [`crate::PrimitiveTopology::Triangles`], `TriangleStrip`, `Points`, `DebugLines` or
/// `DebugLineStrip`: `Quads`, `Lines` and `TriangleFan` are expanded from a vertex range the
/// engine knows, which an indirect command does not give it.
///
/// A command's `first_instance` is read as written only where
/// [`crate::RenderCapabilities::indirect_first_instance`] is set; elsewhere keep it zero.
///
/// An indexed command's `base_vertex` offsets the vertex fetch and `gl_VertexID`; the engine's
/// daily GPU test draws a command with a non-zero `base_vertex` on the CI adapter.
#[derive(Clone, Debug)]
#[non_exhaustive]
// no wire: local client indirect draw descriptor.
pub struct IndirectDraw<'a> {
    /// The pipeline that draws.
    pub pipeline: RenderPipelineRef,
    /// The vertex and index buffers to bind. `None` for a pipeline that declares no vertex
    /// bindings and pulls its geometry from a storage buffer in [`Self::bindings`].
    pub mesh: Option<MeshRef>,
    /// The buffer holding the commands. It must have been created with
    /// [`crate::StorageBufferDescriptor::indirect`].
    pub indirect_buffer: StorageBufferRef,
    /// Byte offset of the first command in `indirect_buffer`: a multiple of 4, with
    /// `indirect_offset + max_count * stride` inside the buffer.
    pub indirect_offset: u64,
    /// How many commands are drawn from `indirect_offset`, or with a count buffer the most
    /// that are. Zero is a legal no-op.
    pub max_count: u32,
    /// The buffer and byte offset of a `u32` the GPU wrote: how many commands to draw, capped
    /// by `max_count`. The buffer must have been created with
    /// [`crate::StorageBufferDescriptor::indirect`]; the offset is a multiple of 4 with its 4
    /// bytes inside the buffer. `None` draws `max_count` commands.
    pub count_buffer: Option<(StorageBufferRef, u64)>,
    /// The pipeline's set-1 bindings for the draw.
    pub bindings: &'a DrawBindings<'a>,
}

impl<'a> IndirectDraw<'a> {
    /// `max_count` commands from offset 0 of `indirect_buffer`, drawn by `pipeline`.
    pub const fn new(
        pipeline: RenderPipelineRef,
        indirect_buffer: StorageBufferRef,
        max_count: u32,
        bindings: &'a DrawBindings<'a>,
    ) -> Self {
        Self {
            pipeline,
            mesh: None,
            indirect_buffer,
            indirect_offset: 0,
            max_count,
            count_buffer: None,
            bindings,
        }
    }

    /// Bind `mesh`'s vertex buffers and, if it has one, its index buffer.
    pub const fn with_mesh(mut self, mesh: MeshRef) -> Self {
        self.mesh = Some(mesh);
        self
    }

    /// Start at byte `offset` of the buffer.
    pub const fn with_offset(mut self, offset: u64) -> Self {
        self.indirect_offset = offset;
        self
    }

    /// Draw as many commands as the `u32` at byte `offset` of `buffer` says, at most
    /// `max_count`. Refused with [`crate::PipelineDrawError::UnsupportedIndirectCount`] where
    /// [`crate::RenderCapabilities::multi_draw_indirect_count`] is false.
    pub const fn with_count_buffer(mut self, buffer: StorageBufferRef, offset: u64) -> Self {
        self.count_buffer = Some((buffer, offset));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draw_starts_at_the_front_of_its_buffer_without_a_mesh() {
        let pipeline = RenderPipelineRef(1, 0);
        let buffer = StorageBufferRef::from_parts(2, 0).unwrap();
        let mesh = MeshRef::from_parts(3, 1).unwrap();
        let bindings = DrawBindings::new();
        let draw = IndirectDraw::new(pipeline, buffer, 8, &bindings);
        assert_eq!(
            (draw.mesh, draw.indirect_offset, draw.max_count),
            (None, 0, 8)
        );
        let draw = draw.with_mesh(mesh).with_offset(40);
        assert_eq!((draw.mesh, draw.indirect_offset), (Some(mesh), 40));
    }
}
