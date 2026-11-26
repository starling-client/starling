//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_render` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ClientRenderClientHost {
    /// Side: client.
    /// `ItemStackRenderState.usesBlockLight` of `stack`'s model resolved for `context`
    /// (`ItemModelResolver.updateForTopItem`, no owner or level): whether the model is lit as a
    /// block is, from the side (`gui_light: side`, vanilla's 3D item models), where a flat item
    /// is lit from the front. It is what the 1.21.3 `BakedModel.isGui3d` became: a mod uses it to
    /// draw a stack as a flat sprite or as a 3D model. False for an
    /// empty or invalid stack, and for a model that draws nothing. Reads the model the client
    /// resolved from its resource packs.
    fn item_uses_block_light(&self, stack: &ItemStack, context: ItemDisplayContext) -> bool;
    /// Side: client.
    /// Resolve `stack`'s item model once for `display` and `seed`
    /// (`ItemModelResolver.updateForTopItem`, no owner), returning a retained [`PreparedItemRef`]
    /// for [`BlockEntityDrawContext::prepared_item`]. Returns [`ModelError::UnknownModel`] for
    /// an empty stack, an unknown/invalid stack, or a model that draws nothing. The handle goes
    /// stale on a successful resource reload, world close, owner mod disable, or
    /// [`PreparedItemRef::destroy`].
    fn prepare_item(
        &self,
        stack: &ItemStack,
        display: ItemDisplayContext,
        seed: i32,
    ) -> Result<PreparedItemRef, ModelError>;
    /// Side: client.
    /// Asks `block_render_visibility` again for every state and, when the answers differ
    /// from those in use, remeshes every loaded section with them. While a mod listens to
    /// `section_visibility`, rebuilds meshes in the player's current level and asks about them
    /// again, except for a first all-air compile with no section geometry contributors. Other
    /// viewed levels do not call that hook; no NeoForge counterpart has been verified. Call it
    /// when a decision changes (a toggle key, a scan finishing), not every frame. With no part
    /// answering or listener, the mesher keeps its no-mod path.

    fn refresh_render_visibility(&mut self);
    /// Side: client.
    /// Marks the loaded chunk section `section` (section coordinates: block X, Y and Z shifted
    /// right by 4) for remeshing, as `LevelExtractor.setSectionDirty` does; with `with_neighbors`
    /// each loaded section of the 27 from one below to one above it on every axis, whether or
    /// not `section` itself is loaded (`ClientLevel.setSectionDirtyWithNeighbors`).
    /// Use it when your mod changes how blocks look (a tint or texture toggle, an overlay) and
    /// the meshes must follow, without unloading chunks. Returns whether `section` itself was
    /// marked: `false` when it is not loaded or is outside the level's height.
    fn set_section_dirty(&mut self, section: [i32; 3], with_neighbors: bool) -> bool;
    /// Side: client.
    /// Marks for remeshing every loaded chunk section that holds a block of the box from
    /// `min - 1` to `max + 1` (block positions, both corners included: the box widened by one
    /// block on each axis), as `LevelExtractor.setBlocksDirty` does, so a box touching a
    /// section's edge also marks the section across it. Sections that are not loaded, or
    /// outside the level's height, are skipped.
    fn set_blocks_dirty(&mut self, min: [i32; 3], max: [i32; 3]);
    /// Side: client.
    /// Visits the chunk columns of the level the client shows whose sections the level renderer
    /// has queued for meshing, `[x, z]` in chunk coordinates (block X and Z shifted right by 4),
    /// in no particular order: the loaded footprint of vanilla terrain, which a distant-terrain
    /// mod rasterizes to meet it without a seam. A column joins once its sections are queued for
    /// meshing after it loads, and leaves when it unloads or falls past the render distance.
    /// Its meshes may still be in flight for a frame or two after it joins. Every section of a
    /// listed column, from the level's bottom to its top, is in the footprint.
    /// Return `ControlFlow::Break(())` to stop; the walk returns what the last call returned,
    /// `Continue(())` when it visited every column, including none. Nothing is visited before a
    /// level is shown.
    fn visit_rendered_chunk_columns(
        &self,
        visit: &mut dyn FnMut([i32; 2]) -> core::ops::ControlFlow<()>,
    ) -> core::ops::ControlFlow<()>;
    /// Side: client.
    /// Asks `biome_colors` again for every biome and, when the colours
    /// differ from those in use, rebuilds every loaded section and the far
    /// terrain with them (`LevelRenderer.allChanged`, as changing Biome Blend
    /// does). Call it when your colours change (a new season, a new day of a
    /// slow fade) and once when the player joins, not every frame: a rebuild
    /// takes the workers a few seconds.
    fn refresh_biome_colors(&mut self);
    /// Side: client.
    /// Vanilla's `TextureManager.register` of a `DynamicTexture` under `id`, then its `upload`:
    /// the calling mod's RGBA texture `id`, in its own namespace (`mymod:dynamic/panel`), made
    /// from `rgba`, `width * height` pixels of four bytes, row by row from the top left. The
    /// texture is a GPU texture of its own, as `DynamicTexture` makes one: `RGBA8_UNORM` (the
    /// sRGB format is not offered: vanilla's dynamic textures and block atlas are UNORM),
    /// sampled nearest with repeat by the GUI, with no choice of format. It has `mip_levels`
    /// levels: 1 is vanilla's `DynamicTexture` and the default choice, and up to the full chain
    /// (`32 - max(width, height).leading_zeros()`) is allowed. `rgba` fills level 0 only; the
    /// other levels start empty and are filled from the CPU, as vanilla's `MipmapGenerator` does
    /// for the block atlas, with one
    /// [`update_dynamic_texture_region`](Self::update_dynamic_texture_region) per level (no
    /// mipmap is generated on the GPU). Draw it with
    /// [`GuiCanvas::texture`] under `id` (`region.tex` its size), from the next frame on, always
    /// sampling level 0; a mod pipeline binds it by id (`TextureSource::DynamicTexture`) and
    /// reads every level through a [`Sampler`] whose `max_lod` allows them. Any mod
    /// may draw the id, only the owner changes it. It survives resource reloads (F3+T, a pack
    /// change) and a world closing, and lasts until
    /// [`release_dynamic_texture`](Self::release_dynamic_texture) or the mod's unload; if the game loses its graphics device the texture is gone and the next update reads
    /// [`DynamicTextureError::Unknown`], so create it again.
    ///
    /// The game keeps no copy of the pixels, and there is no count limit. Refused by name, with
    /// nothing changed and nothing allocated, as [`DynamicTextureError`] lists: `Unavailable`
    /// when nothing draws here, `NotOwned` for an id outside the mod's owned namespaces (its
    /// manifest namespaces or a content package it serves via `served_by`), `BadSize` for a side
    /// of 0 or pixels that are not `width * height * 4` bytes, `TooLarge` for a side past the
    /// device's largest 2D texture, `BadMipLevels` for `mip_levels` of 0 or past the full
    /// chain, and `NameTaken` for an id that already names a dynamic texture. Running out of
    /// GPU memory is not seen here: the frame that makes the texture logs it, and the id draws
    /// vanilla's missing texture.
    fn create_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        mip_levels: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError>;
    /// Side: client.
    /// Vanilla's `DynamicTexture.upload` of new pixels: the calling mod's texture `id` takes
    /// `rgba`, `width * height` pixels of four bytes, from the next frame on. A different size
    /// makes a new texture under the same id, as registering a new `DynamicTexture` under an id
    /// does, keeping its `mip_levels` (the new size's full chain must hold them, else
    /// `BadMipLevels`), with level 0 holding `rgba` and the other levels empty. Writes apply in
    /// the order they were made, none merged. Send one when the texture
    /// changed, not every frame; for a part of it, use
    /// [`update_dynamic_texture_region`](Self::update_dynamic_texture_region).
    ///
    /// Refused by name, with nothing changed, as [`DynamicTextureError`] lists: `Unavailable`,
    /// `NotOwned`, `Unknown` for an id that names no texture, `BadSize` and `TooLarge`.
    fn update_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError>;
    /// Side: client.
    /// `CommandEncoder.writeToTexture`'s region overload on the calling mod's texture `id`:
    /// `rgba`, `size[0] * size[1]` pixels of four bytes, replaces the region of mip level
    /// `mip_level` (0 is the full-size image) whose top left pixel is `origin`, from the next
    /// frame on, applied in the order the writes were made. `origin` and `size` are in pixels of
    /// that level, whose side is the texture's side shifted right by `mip_level`, at least 1.
    /// The rest of the texture is kept.
    ///
    /// Refused by name, with nothing changed, as [`DynamicTextureError`] lists: `Unavailable`,
    /// `NotOwned`, `Unknown`, `BadMipLevels` for a `mip_level` the texture does not have,
    /// `BadSize` for a side of 0 or pixels that are not `size[0] * size[1] * 4` bytes, and
    /// `OutOfBounds` for a region not inside that level.
    fn update_dynamic_texture_region(
        &mut self,
        id: &Identifier,
        mip_level: u32,
        origin: [u32; 2],
        size: [u32; 2],
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError>;
    /// Side: client.
    /// Vanilla's `TextureManager.release`: releases the calling mod's texture `id`. Its GPU
    /// texture is freed and the id can be created again. A mod's textures are also released when
    /// the mod unloads, and a draw of the id already submitted that frame shows vanilla's missing
    /// texture. Refused by name, with nothing changed: `Unavailable`, `NotOwned` for an
    /// id outside the mod's namespace and `Unknown` for one that names no texture.
    fn release_dynamic_texture(&mut self, id: &Identifier) -> Result<(), DynamicTextureError>;
    /// The resolver's blended colour at `pos` in the current client level, as opaque ARGB.
    /// Like `ClientLevel.getBlockTint` (`BiomeColors.getAverageGrassColor` and its counterparts),
    /// this samples a square at the block's height using the client's biome blend radius and the
    /// same biome colours as the mesher, including updates from [`Self::refresh_biome_colors`].
    /// Use [`ColorResolver::Grass`] or [`ColorResolver::Foliage`] without looking up a block state or
    /// a model. For a specific block or fluid face, use [`Self::block_tint_color`].
    /// Returns [`LevelReadError::Unavailable`] outside the World phase or when the host has
    /// no client level to read.
    fn biome_tint(&self, pos: [i32; 3], resolver: ColorResolver) -> Result<u32, LevelReadError>;
    /// Side: client.
    /// For a client mod drawing a block's faces itself (a copycat drawing its material's, from
    /// `block_sprites`): the 0xRRGGBB colour a face of `state` with tint index `tint_index`
    /// (`FaceSprite::tint_index`) is multiplied by at `pos`, as the chunk mesher tints it there
    /// (biome blend included). Grass, foliage and water use the client's current biome colours,
    /// including updates from [`Self::refresh_biome_colors`]. A face's own tint source or a pack
    /// colormap can give it a different colour. Returns `None` for an untinted face (-1, or an
    /// index past the state's tints). For a biome colour independent of the face's tint source,
    /// use [`Self::biome_tint`].
    fn block_tint_color(&self, state: BlockState, tint_index: i32, pos: [i32; 3]) -> Option<u32>;
    /// Side: client.
    /// The standalone models (resolved with [`RenderAssets::model`]) drawn as part of the block
    /// at `pos` in this client's level, as NeoForge's `ModelData` lets a block's model choose
    /// parts at run time without a block state for each choice: a cable's per-side channel
    /// counts. The chunk mesher draws them after the block's own models, unturned from the
    /// block's lower north-west corner (with its offset), in the block's own light and tints.
    /// Each quad takes its render layer as a block's would (cut out, translucent), and a face
    /// with a cull direction is culled by the neighbour there as the block's own faces are. On
    /// air they draw nothing.
    ///
    /// `models` replaces the list the position had; an empty list clears it. The section is
    /// meshed again when the list changes. The list is the client's own, never saved or sent:
    /// it is cleared when the block state at `pos` or the player's level changes. A resend of
    /// a resident column preserves attachments at unchanged cells; unloading the column clears
    /// them. A block entity is not needed. The attachment pass reads prepared mod tint colours
    /// instead of calling providers on its workers.
    /// After the block geometry, extra faces are drawn in block and list order. Drawing stops
    /// before the next face whose worst-case expansion cannot fit the section's upload budget,
    /// so the last model may be partially drawn.
    ///
    /// A handle a resource reload made stale draws nothing until the list is set again from the
    /// new handles. The reload and setup contexts do not expose this setter. Resolve new handles
    /// in `client_resources_reloaded` with [`RenderAssets::model`] and set the lists again from a
    /// live client callback, such as `client_tick_pre`.
    ///
    /// A refused call changes nothing: `NotLoaded` means the position is outside a resident
    /// column or the level height; `Model` means the handle is stale or is not a registered
    /// standalone model.
    fn set_block_extra_models(
        &mut self,
        pos: [i32; 3],
        models: &[ModelRef],
    ) -> Result<(), BlockExtraModelsError>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: what each resolved block state draws on its six sides, preserving
    /// its properties without reparsing names. Resolve states once at load. Each face's sprite (array and
    /// layer, as `WorldDrawn::block_sprites` cuts them) and its texture by the name `cube` and `quad` take,
    /// where it lies on the side, how its texture runs across it (`FaceSprite::quad_on` draws it on another
    /// block's box, turned as its model turns it), its tint source and index (`block_tint_color`) and
    /// whether it is cut out, translucent or animated; its particle sprite; whether it is a plain
    /// cube. None for a block the game does not know. The model is the state's first random variant. API
    /// 380.
    fn block_sprites(&self, blocks: &[BlockState]) -> Vec<Option<BlockSprites>>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: each texture's sprite by name (`minecraft:block/stone`, `block/stone`) in the blocks
    /// atlas, none for one it lacks. The blocks atlas holds the sprites block models name and every sprite a
    /// mod's sprite source made in it, though no model names it; a sprite only the items atlas lists is not
    /// in it. API 380.
    fn texture_sprites(&self, textures: &[&str]) -> Vec<Option<SpriteRef>>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: where GUI sprite `sprite` lies in the GUI atlas and its size, none for one the atlas lacks
    /// or whose entry its definition removed (a server's metrics-only atlas lacks them all). The atlas names its
    /// sprites by GUI id, the id its definition lists them under: `minecraft:container/slot` (also given as
    /// `container/slot`), a mod's `mymod:panel`; a sprite a registered sprite source made in `gui.json` is found
    /// by the id it named; a client mod's dynamic texture is not a GUI sprite (draw it with `GuiCanvas::texture`).
    fn gui_sprite(&self, sprite: &Identifier) -> Option<AtlasRegion>;
    /// Side: client.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// For a client mod: changes each time the resources load (a resource pack turned on or off can move every
    /// sprite): ask `native_block_sprites` again then. API 380.
    fn block_sprites_version(&self) -> u64;
    /// Side: client.
    /// Allocate retained GPU vertex buffer(s) and an optional index buffer (`GpuBuffer`),
    /// returning an owner-checked generational [`MeshRef`] handle. Buffer sizes must be non-zero,
    /// stride/index/4-byte aligned, and at most `device.limits().max_buffer_size`. Handles are
    /// retired automatically on mod disable, world close, resource reload, or [`MeshRef::release`].
    fn create_mesh(&self, desc: &MeshDescriptor<'_>) -> Result<MeshRef, MeshError>;
    /// Side: client.
    /// Allocate a retained GPU std140 uniform buffer initialized with `bytes` (non-empty, 4-byte
    /// aligned, and at most `device.limits().max_uniform_buffer_binding_size`), returning an
    /// owner-checked generational [`UniformBufferRef`] handle. Retired automatically on mod
    /// disable, world close, resource reload, or [`UniformBufferRef::release`].
    fn create_uniform_buffer(&self, bytes: &[u8]) -> Result<UniformBufferRef, BufferError>;
    /// Side: client.
    /// Allocate a retained GPU storage buffer (vanilla's `GpuBuffer`), returning an owner-checked
    /// generational [`StorageBufferRef`]. `desc.size` is at least 4, a multiple of 4 and at most
    /// `device.limits().max_buffer_size`; a binding takes a range of at most
    /// `max_storage_buffer_binding_size` bytes ([`StorageBufferRef::slice`]). The buffer survives
    /// a resource reload and a world close, and is freed by [`StorageBufferRef::release`] or when
    /// the mod unloads. Refused, never panicking, for a size the device cannot hold.
    fn create_storage_buffer(
        &self,
        desc: &StorageBufferDescriptor,
    ) -> Result<StorageBufferRef, BufferError>;
    /// Side: client.
    /// Allocate a mod-owned offscreen GPU render target (`RenderTarget` / `TextureTarget`),
    /// returning an owner-checked generational [`RenderTargetRef`] handle.
    ///
    /// Size is validated against `1..=device.limits().max_texture_dimension_2d`, colour
    /// attachments against `0..=MAX_COLOR_TARGETS` (with at least one colour or depth
    /// attachment), `mip_levels` against `1..=RenderTargetDescriptor::max_mip_levels(size)` and
    /// a requested storage access against the adapter ([`RenderCapabilities`]): all before any
    /// GPU call, and the allocation itself sits in error scopes, so a refusal is an `Err`.
    /// Contents are retained across frames until reallocated by [`RenderTargetRef::resize`]. A
    /// target survives a resource reload and a world close, and is freed by
    /// [`RenderTargetRef::release`] or when its mod unloads. A device refusal (out of memory or
    /// a validation failure) is [`RenderTargetError::Device`].
    fn create_render_target(
        &self,
        desc: &RenderTargetDescriptor,
    ) -> Result<RenderTargetRef, RenderTargetError>;
    /// Side: client.
    /// The optional device features render code can use, read from the adapter the game runs on
    /// ([`RenderCapabilities`]).
    fn render_capabilities(&self) -> RenderCapabilities;
}

/// The retained instance-group and mesh operations behind [`InstanceGroupRef`] and [`MeshRef`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ClientRenderClientBackend {
    /// Atomically overwrite retained model copies; called through [`InstanceGroupRef::write`].
    #[manual_handle]
    fn instance_group_write(
        &self,
        group: InstanceGroupRef,
        first: u32,
        instances: &[ModelInstance],
    ) -> Result<(), GroupError>;
    /// Set the drawn length; called through [`InstanceGroupRef::set_len`].
    #[manual_handle]
    fn instance_group_set_len(&self, group: InstanceGroupRef, len: u32) -> Result<(), GroupError>;
    /// Replace the group pose; called through [`InstanceGroupRef::set_group_transform`].
    #[manual_handle]
    fn instance_group_set_transform(
        &self,
        group: InstanceGroupRef,
        transform: [f32; 12],
    ) -> Result<(), GroupError>;
    /// Retire retained model copies; called through [`InstanceGroupRef::destroy`].
    #[manual_handle]
    fn instance_group_destroy(&self, group: InstanceGroupRef) -> Result<(), GroupError>;
    /// Retire a prepared item render state; called through [`PreparedItemRef::destroy`].
    #[manual_handle]
    fn prepared_item_destroy(&self, item: PreparedItemRef) -> Result<(), ModelError>;
    /// Partially or fully update a retained mesh's vertex buffer; called through
    /// [`MeshRef::write_vertices`].
    #[manual_handle]
    fn mesh_write_vertices(
        &self,
        mesh: MeshRef,
        binding: u8,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError>;
    /// Partially or fully update a retained mesh's index buffer; called through
    /// [`MeshRef::write_indices`].
    #[manual_handle]
    fn mesh_write_indices(
        &self,
        mesh: MeshRef,
        byte_offset: u64,
        data: &[u8],
    ) -> Result<(), MeshError>;
    /// Reallocate a retained mesh's GPU buffers in place; called through [`MeshRef::replace`].
    #[manual_handle]
    fn mesh_replace(&self, mesh: MeshRef, desc: &MeshDescriptor<'_>) -> Result<(), MeshError>;
    /// Free a retained mesh's GPU buffers and retire the handle; called through
    /// [`MeshRef::release`].
    #[manual_handle]
    fn mesh_release(&self, mesh: MeshRef) -> Result<(), MeshError>;
    /// Partially or fully update a retained uniform buffer; called through
    /// [`UniformBufferRef::write`].
    #[manual_handle]
    fn uniform_buffer_write(
        &self,
        buffer: UniformBufferRef,
        byte_offset: u32,
        bytes: &[u8],
    ) -> Result<(), BufferError>;
    /// Free a retained uniform buffer and retire the handle; called through
    /// [`UniformBufferRef::release`].
    #[manual_handle]
    fn uniform_buffer_release(&self, buffer: UniformBufferRef) -> Result<(), BufferError>;
    /// Queue `data` for `offset` of a retained storage buffer (`CommandEncoder.writeToBuffer` on a
    /// `GpuBufferSlice`; called through [`StorageBufferRef::write_range`]); `offset` and `data.len()` are multiples of 4 and the range lies inside
    /// the buffer. Writes are applied in call order before the next frame, as updates to dynamic
    /// textures are.
    #[manual_handle]
    fn storage_buffer_write_range(
        &self,
        buffer: StorageBufferRef,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BufferError>;
    /// The size in bytes the storage buffer was created with; called through
    /// [`StorageBufferRef::size`].
    #[manual_handle]
    fn storage_buffer_size(&self, buffer: StorageBufferRef) -> Result<u64, BufferError>;
    /// Free a retained storage buffer and retire the handle ([`BufferError::Stale`] thereafter);
    /// called through [`StorageBufferRef::release`].
    /// Writes queued for it and not yet applied are dropped.
    #[manual_handle]
    fn storage_buffer_release(&self, buffer: StorageBufferRef) -> Result<(), BufferError>;
    /// Record a copy of a storage buffer range into a staging buffer at this point of the
    /// compute node being recorded; called through [`StorageBufferRef::request_readback`].
    #[manual_handle]
    fn storage_buffer_request_readback(
        &self,
        buffer: StorageBufferRef,
        offset: u64,
        len: u64,
    ) -> Result<ReadbackTicket, BufferError>;
    /// The bytes of a finished readback, none while it is in flight; called through
    /// [`ReadbackTicket::poll_readback`].
    #[manual_handle]
    fn readback_poll(&self, ticket: ReadbackTicket) -> Result<Option<Vec<u8>>, BufferError>;
    /// Drop a pending readback; called through [`ReadbackTicket::release`].
    #[manual_handle]
    fn readback_release(&self, ticket: ReadbackTicket) -> Result<(), BufferError>;
    /// Resize a mod-owned offscreen render target (no-op if unchanged); called through
    /// [`RenderTargetRef::resize`].
    #[manual_handle]
    fn render_target_resize(
        &self,
        target: RenderTargetRef,
        size: [u32; 2],
    ) -> Result<(), RenderTargetError>;
    /// Query a mod-owned offscreen render target's pixel dimensions; called through
    /// [`RenderTargetRef::size`].
    #[manual_handle]
    fn render_target_size(&self, target: RenderTargetRef) -> Result<[u32; 2], RenderTargetError>;
    /// Free a mod-owned offscreen render target and retire the handle; called through
    /// [`RenderTargetRef::release`].
    #[manual_handle]
    fn render_target_release(&self, target: RenderTargetRef) -> Result<(), RenderTargetError>;
}
