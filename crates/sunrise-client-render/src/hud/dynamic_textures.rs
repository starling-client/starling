//! Client mods' dynamic textures (`ClientRenderClientHost::create_dynamic_texture`),
//! vanilla's `DynamicTexture` registered in the `TextureManager` under an id
//! (`TextureManager.register`, `release`). Each texture is a GPU texture of its own
//! (`RGBA8_UNORM`, one mip level unless the mod asks for more, sampled nearest with repeat by the
//! GUI: `DynamicTexture`'s constructor), drawn as a standalone texture, from level 0, by
//! `HudBatch::dynamic_texture`.
//!
//! Two halves meet through [`Command`]s. [`DynamicTextures`] is the client thread's: which ids
//! exist, who owns them and how large they are, with every refusal decided there, and no pixels
//! kept. Each accepted call queues a command; a frame takes the queue
//! ([`DynamicTextures::take_commands`]) in the batch, and the render thread applies the commands
//! in order before the frame ([`Store::apply`]), with no merging. [`Store`] holds the GPU
//! textures. It is shared by the renderers that follow one another on the same device, so a
//! resource reload keeps every texture; a new device starts with an empty one, which
//! [`DynamicTextures::bind`] notices and answers by forgetting every id.
use mod_api::DynamicTextureError;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use sunrise_core::identifier::Identifier;

/// One change to the render thread's textures, applied in the order the calls were made.
/// `handle` names a texture for as long as its id lives, whatever size it is re-created at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// A new texture of `mip_levels` levels, or one replacing the handle's (a different size),
    /// with `rgba` in level 0 and the other levels empty.
    Create {
        handle: u64,
        size: [u32; 2],
        mip_levels: u32,
        rgba: Vec<u8>,
    },
    /// `CommandEncoder.writeToTexture`'s region overload: `rgba` into the region at `origin` of
    /// mip level `mip_level`.
    Write {
        handle: u64,
        mip_level: u32,
        origin: [u32; 2],
        size: [u32; 2],
        rgba: Vec<u8>,
    },
    /// `TextureManager.release`.
    Release { handle: u64 },
    /// A GPU copy of the whole texture into the HUD atlas at `dst`: how a skin a mod selects
    /// reaches the atlas slot the body, hand and preview sample.
    CopyToAtlas { handle: u64, dst: [u32; 2] },
}

fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The client thread's table of dynamic textures.
#[derive(Default)]
pub struct DynamicTextures {
    images: BTreeMap<Identifier, Image>,
    queue: Vec<Command>,
    /// The [`Store`] these ids are in, and the device's largest 2D texture side; none until a
    /// renderer is bound.
    bound: Option<(u64, u32)>,
    geometry_revision: u64,
    /// `mods_switched` as of the last [`Self::release_unloaded`].
    switched: u64,
}

struct Image {
    /// The native mod that created this texture.
    owner: Box<str>,
    handle: u64,
    size: [u32; 2],
    mip_levels: u32,
}

/// The length of a full mip chain for a texture of `size`: `32 - max(w, h).leading_zeros()`.
fn full_chain(size: [u32; 2]) -> u32 {
    32 - size[0].max(size[1]).leading_zeros()
}

/// The side of mip level `level` of a texture side `side`, at least 1.
fn level_side(side: u32, level: u32) -> u32 {
    side.checked_shr(level).unwrap_or(0).max(1)
}

/// `BadSize` for a side of 0 or pixels that are not `width * height * 4` bytes, `TooLarge` for a
/// side past `max`. The side is compared before the bytes are counted, so the count only sees
/// sides of at most `max` and, counted in `u64`, cannot overflow.
fn checked(size: [u32; 2], len: usize, max: u32) -> Result<(), DynamicTextureError> {
    if size.contains(&0) {
        return Err(DynamicTextureError::BadSize);
    }
    if size.iter().any(|side| *side > max) {
        return Err(DynamicTextureError::TooLarge);
    }
    if u64::from(size[0]) * u64::from(size[1]) * 4 != len as u64 {
        return Err(DynamicTextureError::BadSize);
    }
    Ok(())
}

impl DynamicTextures {
    /// Changes when a texture is added, removed or resized; pixel-only updates don't relayout text.
    pub fn geometry_revision(&self) -> u64 {
        self.geometry_revision
    }

    /// The renderer's `store` and its device's `max_side`, read once per renderer. A store that
    /// is not the one the ids were made in is a new device: every id is forgotten and every
    /// queued command dropped, so the owning mods read `Unknown` and create again.
    pub fn bind(&mut self, store: u64, max_side: u32) {
        if self.bound.map(|(id, _)| id) != Some(store) {
            self.images.clear();
            self.queue.clear();
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
        }
        self.bound = Some((store, max_side));
    }

    /// Mod `owner` creates texture `id` of `size` and `mip_levels` levels, level 0 from `rgba`.
    /// Every refusal comes before the pixels are copied, and changes nothing.
    pub fn create(
        &mut self,
        owner: &str,
        id: &Identifier,
        size: [u32; 2],
        mip_levels: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        let (_, max) = self.bound.ok_or(DynamicTextureError::Unavailable)?;
        checked(size, rgba.len(), max)?;
        if mip_levels == 0 || mip_levels > full_chain(size) {
            return Err(DynamicTextureError::BadMipLevels);
        }
        if self.images.contains_key(id) {
            return Err(DynamicTextureError::NameTaken);
        }
        let handle = next_id();
        self.queue.push(Command::Create {
            handle,
            size,
            mip_levels,
            rgba: rgba.to_vec(),
        });
        self.images.insert(
            id.clone(),
            Image {
                owner: owner.into(),
                handle,
                size,
                mip_levels,
            },
        );
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
        Ok(())
    }

    /// Mod `owner` gives its texture `id` new pixels: the same size writes the whole texture, a
    /// different one makes a new texture under the id's handle.
    pub fn update(
        &mut self,
        owner: &str,
        id: &Identifier,
        size: [u32; 2],
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        let (_, max) = self.bound.ok_or(DynamicTextureError::Unavailable)?;
        let image = self.owned_image(owner, id)?;
        checked(size, rgba.len(), max)?;
        let handle = image.handle;
        let rgba = rgba.to_vec();
        if image.size == size {
            self.queue.push(Command::Write {
                handle,
                mip_level: 0,
                origin: [0, 0],
                size,
                rgba,
            });
        } else {
            let mip_levels = image.mip_levels;
            if mip_levels > full_chain(size) {
                return Err(DynamicTextureError::BadMipLevels);
            }
            image.size = size;
            self.queue.push(Command::Create {
                handle,
                size,
                mip_levels,
                rgba,
            });
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
        }
        Ok(())
    }

    /// Mod `owner` replaces the region of mip level `mip_level` of its texture `id` at `origin`
    /// of `size` (both in pixels of that level) with `rgba`.
    pub fn update_region(
        &mut self,
        owner: &str,
        id: &Identifier,
        mip_level: u32,
        origin: [u32; 2],
        size: [u32; 2],
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        let (_, max) = self.bound.ok_or(DynamicTextureError::Unavailable)?;
        let image = self.owned_image(owner, id)?;
        if mip_level >= image.mip_levels {
            return Err(DynamicTextureError::BadMipLevels);
        }
        let (handle, extent) = (
            image.handle,
            [
                level_side(image.size[0], mip_level),
                level_side(image.size[1], mip_level),
            ],
        );
        checked(size, rgba.len(), max)?;
        let inside = |axis: usize| {
            origin[axis]
                .checked_add(size[axis])
                .is_some_and(|end| end <= extent[axis])
        };
        if !(inside(0) && inside(1)) {
            return Err(DynamicTextureError::OutOfBounds);
        }
        self.queue.push(Command::Write {
            handle,
            mip_level,
            origin,
            size,
            rgba: rgba.to_vec(),
        });
        Ok(())
    }

    /// Mod `owner` releases its texture `id`.
    pub fn release(&mut self, owner: &str, id: &Identifier) -> Result<(), DynamicTextureError> {
        self.bound.ok_or(DynamicTextureError::Unavailable)?;
        let handle = self.owned_image(owner, id)?.handle;
        self.images.remove(id);
        self.queue.push(Command::Release { handle });
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
        Ok(())
    }

    /// Mod `owner` unloaded: every texture it made is released.
    pub fn release_owned_by(&mut self, owner: &str) {
        let queue = &mut self.queue;
        let before = self.images.len();
        self.images.retain(|_, image| {
            let keep = image.owner.as_ref() != owner;
            if !keep {
                queue.push(Command::Release {
                    handle: image.handle,
                });
            }
            keep
        });
        if self.images.len() != before {
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
        }
    }

    /// When the mods were switched since the last call (`switched` changed), releases the
    /// textures of every mod `loaded` no longer holds on.
    pub fn release_unloaded(&mut self, switched: u64, loaded: impl Fn(&str) -> bool) {
        if std::mem::replace(&mut self.switched, switched) == switched {
            return;
        }
        let gone: Vec<Box<str>> = self
            .images
            .values()
            .map(|image| image.owner.clone())
            .filter(|owner| !loaded(owner))
            .collect();
        for owner in gone {
            self.release_owned_by(&owner);
        }
    }

    fn owned_image(
        &mut self,
        owner: &str,
        id: &Identifier,
    ) -> Result<&mut Image, DynamicTextureError> {
        let image = self
            .images
            .get_mut(id)
            .ok_or(DynamicTextureError::Unknown)?;
        if image.owner.as_ref() != owner {
            return Err(DynamicTextureError::NotOwned);
        }
        Ok(image)
    }

    /// The handle and size of the live texture `id`, if this id names one.
    pub fn get(&self, id: &Identifier) -> Option<(u64, [u32; 2])> {
        self.images.get(id).map(|image| (image.handle, image.size))
    }
    /// The dimensions of a live texture, if this id names one.
    pub fn size(&self, id: &Identifier) -> Option<[u32; 2]> {
        self.get(id).map(|(_, size)| size)
    }

    /// Queue the GPU copy of the live 64 x 64 texture `id` into the atlas at `dst`; false when
    /// `id` names no such texture.
    pub fn copy_to_atlas(&mut self, id: &Identifier, dst: [u32; 2]) -> bool {
        let Some((handle, [64, 64])) = self.get(id) else {
            return false;
        };
        self.queue.push(Command::CopyToAtlas { handle, dst });
        true
    }

    /// The live textures, `(id, handle)` in id order.
    pub fn live(&self) -> impl Iterator<Item = (&Identifier, u64)> {
        self.images.iter().map(|(id, image)| (id, image.handle))
    }

    /// Move the queued commands, in order, into `out` for this frame.
    pub fn take_commands(&mut self, out: &mut Vec<Command>) {
        out.append(&mut self.queue);
    }
}

/// The render thread's textures, by handle.
#[derive(Clone)]
pub struct Store(Arc<StoreInner>);

struct StoreInner {
    id: u64,
    textures: Mutex<HashMap<u64, Gpu>>,
}

/// One texture and the view a bind group samples.
pub(super) struct Gpu {
    texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
    /// Changes when the texture is made again, so a bind group built for the old one is stale.
    pub(super) revision: u64,
}

impl Default for Store {
    fn default() -> Self {
        Self(Arc::new(StoreInner {
            id: next_id(),
            textures: Mutex::default(),
        }))
    }
}

impl Store {
    /// What [`DynamicTextures::bind`] tells apart.
    pub fn id(&self) -> u64 {
        self.0.id
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Gpu>> {
        self.0
            .textures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The live view of texture `handle`, cloned, and its revision (unique per GPU texture made,
    /// so a bind group built for a texture made again is stale), under one lock; `None` when it
    /// has no GPU texture: its creation failed, or the device was lost.
    pub fn view(&self, handle: u64) -> Option<(wgpu::TextureView, u64)> {
        self.lock()
            .get(&handle)
            .map(|gpu| (gpu.view.clone(), gpu.revision))
    }

    /// Under one lock, for each of `drawn` that has no bind group in `held` of its current
    /// revision, push it to `out` with its view and revision, or with `None` when it no longer
    /// exists. A view is cloned only for a bind group that must be made again.
    pub(crate) fn changed(
        &self,
        drawn: &[u64],
        held: &HashMap<u64, (u64, wgpu::BindGroup)>,
        out: &mut Vec<(u64, Option<(wgpu::TextureView, u64)>)>,
    ) {
        let textures = self.lock();
        for &handle in drawn {
            match textures.get(&handle) {
                Some(gpu) if held.get(&handle).is_some_and(|(r, _)| *r == gpu.revision) => {}
                Some(gpu) => out.push((handle, Some((gpu.view.clone(), gpu.revision)))),
                None => out.push((handle, None)),
            }
        }
    }

    /// `DynamicTexture.dumpContents` of each texture in `textures` (`F3 + S`): read back from the
    /// GPU and written to `dir` as a PNG named after its id with `/` and `:` as `_`
    /// (`Identifier.toDebugFileName`). A texture with no GPU texture (its creation failed, or its
    /// commands have not reached the render thread yet) is skipped. Returns the files written.
    pub fn dump_contents<'a>(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: impl IntoIterator<Item = (&'a Identifier, u64)>,
        dir: &std::path::Path,
    ) -> std::io::Result<Vec<std::path::PathBuf>> {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut reads = Vec::new();
        {
            let gpu = self.lock();
            for (id, handle) in textures {
                let Some(gpu) = gpu.get(&handle) else {
                    continue;
                };
                let (width, height) = (gpu.texture.width(), gpu.texture.height());
                let padded = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Dynamic texture dump"),
                    size: u64::from(padded) * u64::from(height),
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                encoder.copy_texture_to_buffer(
                    gpu.texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(padded),
                            rows_per_image: Some(height),
                        },
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
                let name = id.to_string().replace(['/', ':'], "_");
                reads.push((name, width, height, padded, buffer));
            }
        }
        if reads.is_empty() {
            return Ok(Vec::new());
        }
        queue.submit([encoder.finish()]);
        for (_, _, _, _, buffer) in &reads {
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        }
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(std::io::Error::other)?;
        let mut written = Vec::with_capacity(reads.len());
        for (name, width, height, padded, buffer) in reads {
            let data = buffer
                .slice(..)
                .get_mapped_range()
                .map_err(std::io::Error::other)?;
            let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
            for row in data.chunks(padded as usize) {
                pixels.extend_from_slice(&row[..width as usize * 4]);
            }
            written.push(super::atlas::write_dump_png(
                dir, &name, width, height, &pixels,
            )?);
        }
        Ok(written)
    }

    /// Apply `commands` in order into `encoder`, ahead of the frame's draws. `atlas` is the HUD
    /// atlas texture a `CopyToAtlas` writes. A command for a handle that no longer exists, or
    /// outside its texture, is skipped: the client thread only sends what it checked.
    pub fn apply(
        &self,
        device: &wgpu::Device,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        atlas: Option<&wgpu::Texture>,
        commands: &[Command],
    ) {
        let mut textures = self.lock();
        for command in commands {
            match command {
                Command::Create {
                    handle,
                    size,
                    mip_levels,
                    rgba,
                } => {
                    // The sides were checked against the device's limit when the call was made. A
                    // failure the device still reports is logged and leaves no GPU texture, while the
                    // client table keeps the id: it draws vanilla's missing texture
                    // (`prepare_dynamic` draws it for a handle with no view), as an id released after
                    // its draw was submitted does (`TextureManager.release`). A later
                    // `update_dynamic_texture` of a different size creates it again.
                    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Dynamic texture"),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: *mip_levels,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    });
                    let failed = pollster::block_on(out_of_memory.pop())
                        .or(pollster::block_on(validation.pop()));
                    if let Some(error) = failed {
                        log::warn!("dynamic texture {handle} was not created: {error}");
                        textures.remove(handle);
                        continue;
                    }
                    uploads.texture(encoder, texture.as_image_copy(), size[0], size[1], rgba);
                    let view = texture.create_view(&Default::default());
                    textures.insert(
                        *handle,
                        Gpu {
                            texture,
                            view,
                            revision: next_id(),
                        },
                    );
                }
                Command::Write {
                    handle,
                    mip_level,
                    origin,
                    size,
                    rgba,
                } => {
                    let Some(gpu) = textures.get(handle) else {
                        continue;
                    };
                    if *mip_level >= gpu.texture.mip_level_count() {
                        continue;
                    }
                    let (width, height) = (
                        level_side(gpu.texture.width(), *mip_level),
                        level_side(gpu.texture.height(), *mip_level),
                    );
                    if origin[0].saturating_add(size[0]) > width
                        || origin[1].saturating_add(size[1]) > height
                    {
                        continue;
                    }
                    uploads.texture(
                        encoder,
                        wgpu::TexelCopyTextureInfo {
                            texture: &gpu.texture,
                            mip_level: *mip_level,
                            origin: wgpu::Origin3d {
                                x: origin[0],
                                y: origin[1],
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        size[0],
                        size[1],
                        rgba,
                    );
                }
                Command::Release { handle } => {
                    textures.remove(handle);
                }
                Command::CopyToAtlas { handle, dst } => {
                    let (Some(gpu), Some(atlas)) = (textures.get(handle), atlas) else {
                        continue;
                    };
                    let (width, height) = (gpu.texture.width(), gpu.texture.height());
                    if dst[0].saturating_add(width) > atlas.width()
                        || dst[1].saturating_add(height) > atlas.height()
                    {
                        continue;
                    }
                    encoder.copy_texture_to_texture(
                        gpu.texture.as_image_copy(),
                        wgpu::TexelCopyTextureInfo {
                            texture: atlas,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: dst[0],
                                y: dst[1],
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DynamicTextureError::*;

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    /// A table bound to a store whose device allows textures up to 128 pixels a side.
    fn textures() -> DynamicTextures {
        let mut textures = DynamicTextures::default();
        textures.bind(1, 128);
        textures
    }

    fn commands(textures: &mut DynamicTextures) -> Vec<Command> {
        let mut out = Vec::new();
        textures.take_commands(&mut out);
        out
    }

    /// `F3 + S` reads each live texture back from its own GPU texture and writes it under its
    /// id's debug file name (`DynamicTexture.dumpContents`); one the render thread has not made
    /// yet is skipped.
    #[test]
    #[ignore = "daily"]
    fn a_dump_writes_each_live_texture_from_the_gpu() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let store = Store::default();
        let mut textures = DynamicTextures::default();
        textures.bind(store.id(), 128);
        // Three pixels wide: each row is padded to 256 bytes on the GPU.
        let rgba: Vec<u8> = (0..3 * 2 * 4).map(|i| (i * 9) as u8).collect();
        textures
            .create("mod", &id("mod:dyn/icon"), [3, 2], 1, &rgba)
            .unwrap();
        let mut uploads = crate::gfx::upload::Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        store.apply(
            &device,
            &mut uploads,
            &mut encoder,
            None,
            &commands(&mut textures),
        );
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        textures
            .create("mod", &id("mod:pending"), [1, 1], 1, &[0; 4])
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let written = store
            .dump_contents(&device, &queue, textures.live(), dir.path())
            .unwrap();
        assert_eq!(written, [dir.path().join("mod_dyn_icon.png")]);
        let bytes = std::fs::read(&written[0]).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((frame.width, frame.height), (3, 2));
        assert_eq!(&pixels[..frame.buffer_size()], &rgba[..]);
    }

    #[test]
    fn nothing_is_made_before_a_renderer_is_bound() {
        let mut textures = DynamicTextures::default();
        let a = id("mod:a");
        assert_eq!(
            textures.create("mod", &a, [1, 1], 1, &[0; 4]),
            Err(Unavailable)
        );
        assert_eq!(textures.release("mod", &a), Err(Unavailable));
    }

    #[test]
    fn a_pixel_buffer_of_the_wrong_length_or_size_is_refused_by_name() {
        let mut textures = textures();
        let a = id("mod:a");
        assert_eq!(
            textures.create("mod", &a, [2, 2], 1, &[0; 15]),
            Err(BadSize)
        );
        assert_eq!(
            textures.create("mod", &a, [2, 2], 1, &[0; 17]),
            Err(BadSize)
        );
        assert_eq!(textures.create("mod", &a, [0, 4], 1, &[]), Err(BadSize));
        assert_eq!(textures.create("mod", &a, [4, 0], 1, &[]), Err(BadSize));
        assert!(commands(&mut textures).is_empty());
        assert_eq!(textures.get(&a), None);
    }

    #[test]
    fn a_side_past_the_devices_maximum_is_too_large_before_its_length_is_counted() {
        let mut textures = textures();
        let a = id("mod:a");
        assert_eq!(
            textures.create("mod", &a, [129, 1], 1, &[0; 516]),
            Err(TooLarge)
        );
        assert_eq!(textures.create("mod", &a, [1, 129], 1, &[]), Err(TooLarge));
        assert_eq!(
            textures.create("mod", &a, [u32::MAX, u32::MAX], 1, &[]),
            Err(TooLarge)
        );
        assert_eq!(textures.create("mod", &a, [128, 1], 1, &[0; 512]), Ok(()));
    }

    #[test]
    fn a_name_is_taken_unknown_or_another_mods() {
        let mut textures = textures();
        let a = id("mod:a");
        textures.create("mod", &a, [1, 1], 1, &[0; 4]).unwrap();
        assert_eq!(
            textures.create("mod", &a, [1, 1], 1, &[0; 4]),
            Err(NameTaken)
        );
        assert_eq!(
            textures.create("other", &a, [1, 1], 1, &[0; 4]),
            Err(NameTaken)
        );
        let missing = id("mod:missing");
        assert_eq!(
            textures.update("mod", &missing, [1, 1], &[0; 4]),
            Err(Unknown)
        );
        assert_eq!(textures.release("mod", &missing), Err(Unknown));
        assert_eq!(textures.update("other", &a, [1, 1], &[0; 4]), Err(NotOwned));
        assert_eq!(
            textures.update_region("other", &a, 0, [0, 0], [1, 1], &[0; 4]),
            Err(NotOwned)
        );
        assert_eq!(textures.release("other", &a), Err(NotOwned));
        assert_eq!(textures.size(&a), Some([1, 1]));
    }

    #[test]
    fn a_region_outside_the_texture_is_refused_by_name() {
        let mut textures = textures();
        let a = id("mod:a");
        textures.create("mod", &a, [4, 4], 1, &[0; 64]).unwrap();
        commands(&mut textures);
        let region = |textures: &mut DynamicTextures, origin, size: [u32; 2]| {
            let rgba = vec![0; (size[0] * size[1] * 4) as usize];
            textures.update_region("mod", &a, 0, origin, size, &rgba)
        };
        assert_eq!(region(&mut textures, [3, 0], [2, 1]), Err(OutOfBounds));
        assert_eq!(region(&mut textures, [0, 4], [1, 1]), Err(OutOfBounds));
        assert_eq!(
            region(&mut textures, [u32::MAX, 0], [1, 1]),
            Err(OutOfBounds)
        );
        assert_eq!(region(&mut textures, [0, 0], [5, 1]), Err(OutOfBounds));
        assert_eq!(
            textures.update_region("mod", &a, 0, [0, 0], [1, 1], &[0; 3]),
            Err(BadSize)
        );
        assert!(commands(&mut textures).is_empty());
        assert_eq!(region(&mut textures, [2, 3], [2, 1]), Ok(()));
    }

    #[test]
    fn mip_levels_run_from_one_to_the_full_chain_and_a_write_is_checked_against_its_level() {
        let mut textures = textures();
        let a = id("mod:a");
        // A 8 x 4 texture has a full chain of 4 levels: 8x4, 4x2, 2x1, 1x1.
        let base = vec![0; 8 * 4 * 4];
        for bad in [0, 5, u32::MAX] {
            assert_eq!(
                textures.create("mod", &a, [8, 4], bad, &base),
                Err(BadMipLevels)
            );
        }
        assert_eq!(textures.get(&a), None);
        assert_eq!(textures.create("mod", &a, [8, 4], 4, &base), Ok(()));
        commands(&mut textures);
        let write = |textures: &mut DynamicTextures, level, origin, size: [u32; 2]| {
            let rgba = vec![0; (size[0] * size[1] * 4) as usize];
            textures.update_region("mod", &a, level, origin, size, &rgba)
        };
        assert_eq!(write(&mut textures, 1, [0, 0], [4, 2]), Ok(()));
        assert_eq!(write(&mut textures, 1, [0, 0], [5, 2]), Err(OutOfBounds));
        assert_eq!(write(&mut textures, 2, [1, 0], [1, 1]), Ok(()));
        assert_eq!(write(&mut textures, 2, [0, 0], [2, 2]), Err(OutOfBounds));
        assert_eq!(write(&mut textures, 3, [0, 0], [1, 1]), Ok(()));
        assert_eq!(write(&mut textures, 3, [1, 0], [1, 1]), Err(OutOfBounds));
        assert_eq!(write(&mut textures, 4, [0, 0], [1, 1]), Err(BadMipLevels));
        assert_eq!(
            write(&mut textures, u32::MAX, [0, 0], [1, 1]),
            Err(BadMipLevels)
        );
        assert_eq!(
            textures.update_region("mod", &a, 2, [0, 0], [2, 1], &[0; 7]),
            Err(BadSize)
        );
        assert_eq!(
            commands(&mut textures)
                .iter()
                .map(|command| match command {
                    Command::Write { mip_level, .. } => *mip_level,
                    other => panic!("{other:?}"),
                })
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn a_new_size_keeps_the_mip_levels_when_its_chain_holds_them() {
        let mut textures = textures();
        let a = id("mod:a");
        textures.create("mod", &a, [8, 8], 3, &[0; 256]).unwrap();
        commands(&mut textures);
        assert_eq!(
            textures.update("mod", &a, [2, 2], &[0; 16]),
            Err(BadMipLevels)
        );
        assert_eq!(textures.size(&a), Some([8, 8]));
        assert!(commands(&mut textures).is_empty());
        textures.update("mod", &a, [4, 4], &[0; 64]).unwrap();
        let (handle, _) = textures.get(&a).unwrap();
        assert_eq!(
            commands(&mut textures),
            [Command::Create {
                handle,
                size: [4, 4],
                mip_levels: 3,
                rgba: vec![0; 64]
            }]
        );
    }

    #[test]
    fn calls_queue_in_order_and_a_new_size_recreates_the_same_handle() {
        let mut textures = textures();
        let a = id("mod:a");
        textures.create("mod", &a, [2, 2], 1, &[1; 16]).unwrap();
        let (handle, _) = textures.get(&a).unwrap();
        textures
            .update_region("mod", &a, 0, [1, 1], [1, 1], &[2; 4])
            .unwrap();
        textures.update("mod", &a, [2, 2], &[3; 16]).unwrap();
        textures.update("mod", &a, [1, 3], &[4; 12]).unwrap();
        textures.release("mod", &a).unwrap();
        assert_eq!(
            commands(&mut textures),
            [
                Command::Create {
                    handle,
                    size: [2, 2],
                    mip_levels: 1,
                    rgba: vec![1; 16]
                },
                Command::Write {
                    handle,
                    mip_level: 0,
                    origin: [1, 1],
                    size: [1, 1],
                    rgba: vec![2; 4]
                },
                Command::Write {
                    handle,
                    mip_level: 0,
                    origin: [0, 0],
                    size: [2, 2],
                    rgba: vec![3; 16]
                },
                Command::Create {
                    handle,
                    size: [1, 3],
                    mip_levels: 1,
                    rgba: vec![4; 12]
                },
                Command::Release { handle },
            ]
        );
        assert_eq!(textures.get(&a), None);
        assert!(commands(&mut textures).is_empty());
    }

    #[test]
    fn unloading_a_mod_releases_only_its_textures() {
        let mut textures = textures();
        let (a, b) = (id("one:a"), id("two:b"));
        textures.create("one", &a, [1, 1], 1, &[0; 4]).unwrap();
        textures.create("two", &b, [1, 1], 1, &[0; 4]).unwrap();
        let (handle, _) = textures.get(&a).unwrap();
        commands(&mut textures);
        textures.release_owned_by("one");
        assert_eq!(commands(&mut textures), [Command::Release { handle }]);
        assert_eq!(textures.get(&a), None);
        assert!(textures.get(&b).is_some());
    }

    #[test]
    fn a_new_device_forgets_every_texture() {
        let mut textures = textures();
        let a = id("mod:a");
        textures.create("mod", &a, [1, 1], 1, &[0; 4]).unwrap();
        textures.bind(1, 128);
        assert!(textures.get(&a).is_some());
        textures.bind(2, 64);
        assert_eq!(textures.get(&a), None);
        assert!(commands(&mut textures).is_empty());
        assert_eq!(textures.update("mod", &a, [1, 1], &[0; 4]), Err(Unknown));
    }

    #[test]
    fn only_a_64_by_64_texture_copies_into_a_skin_slot() {
        let mut textures = textures();
        let (skin, small) = (id("mod:skin"), id("mod:small"));
        textures
            .create("mod", &skin, [64, 64], 1, &[0; 64 * 64 * 4])
            .unwrap();
        textures
            .create("mod", &small, [32, 64], 1, &[0; 32 * 64 * 4])
            .unwrap();
        commands(&mut textures);
        assert!(!textures.copy_to_atlas(&small, [0, 0]));
        assert!(!textures.copy_to_atlas(&id("mod:none"), [0, 0]));
        assert!(textures.copy_to_atlas(&skin, [8, 16]));
        let (handle, _) = textures.get(&skin).unwrap();
        assert_eq!(
            commands(&mut textures),
            [Command::CopyToAtlas {
                handle,
                dst: [8, 16]
            }]
        );
    }
}
